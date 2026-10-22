use crate::{
    engine::AgentLoop,
    memory::{Entry, Store},
    types::{EngineRequest, EngineRequestMetadata, Message, StreamEvent},
};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct StoredMessage {
    pub id: String,
    pub message: Message,
    pub created_at: String,
}
#[derive(Clone, Debug)]
pub struct Conversation {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<StoredMessage>,
}
#[derive(Clone, Debug)]
pub struct Response {
    pub status: u16,
    pub headers: Map<String, Value>,
    pub body: Value,
    pub stream: Vec<StreamEvent>,
}
pub struct Gateway {
    pub engine: AgentLoop,
    pub memory: Store,
    pub current_model: String,
    conversations: HashMap<String, Conversation>,
}

impl Gateway {
    pub fn new(engine: AgentLoop, memory: Option<Store>) -> Self {
        Self {
            engine,
            memory: memory.unwrap_or_else(|| Store::new(".paa-memory")),
            current_model: "mock".into(),
            conversations: HashMap::new(),
        }
    }
    pub fn handle(
        &mut self,
        method: &str,
        path: &str,
        body: Option<Value>,
        headers: &Map<String, Value>,
    ) -> Response {
        let route = path.split('?').next().unwrap_or(path);
        let method = method.to_ascii_uppercase();
        if method == "GET" && route == "/health" {
            return json_response(200, serde_json::json!({"status":"ok","architecture":"PAA"}));
        }
        if method == "GET" && route == "/conversations" {
            return self.list_conversations();
        }
        if method == "GET" && route.starts_with("/conversations/") {
            return self
                .get_conversation(route.strip_prefix("/conversations/").unwrap_or_default());
        }
        if method == "POST" && (route == "/conversations/messages" || route == "/chat") {
            return self.message(body, headers, "");
        }
        if method == "POST" && route.starts_with("/conversations/") && route.ends_with("/messages")
        {
            let conversation_id = route
                .strip_prefix("/conversations/")
                .and_then(|value| value.strip_suffix("/messages"))
                .unwrap_or_default();
            return self.message(body, headers, conversation_id);
        }
        error_response(404, "not_found", "Route not found.")
    }
    fn message(
        &mut self,
        body: Option<Value>,
        headers: &Map<String, Value>,
        requested_id: &str,
    ) -> Response {
        let Some(Value::Object(body)) = body else {
            return error_response(
                400,
                "invalid_request",
                "Request must contain non-empty content and optional metadata.",
            );
        };
        if body.keys().any(|key| key != "content" && key != "metadata") {
            return error_response(
                400,
                "invalid_request",
                "Request must contain non-empty content and optional metadata.",
            );
        }
        let Some(content) = body
            .get("content")
            .and_then(Value::as_array)
            .filter(|parts| !parts.is_empty())
        else {
            return error_response(
                400,
                "invalid_request",
                "Request must contain non-empty content and optional metadata.",
            );
        };
        if content
            .iter()
            .any(|part| match part.get("type").and_then(Value::as_str) {
                Some("text") => !part.get("text").is_some_and(Value::is_string),
                Some("image" | "file") => !part.get("url").is_some_and(Value::is_string),
                _ => true,
            })
        {
            return error_response(400, "invalid_request", "Invalid content part.");
        }
        let actor = crate::engine::header(headers, "X-Actor-ID");
        let permissions = crate::engine::header(headers, "X-Actor-Permissions");
        if actor.is_empty() || permissions.is_empty() {
            return error_response(
                401,
                "missing_actor_headers",
                "Missing required actor headers.",
            );
        }
        let metadata = body
            .get("metadata")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let conversation_id = if requested_id.is_empty() {
            let id = id("conv");
            let now = timestamp();
            self.conversations.insert(
                id.clone(),
                Conversation {
                    id: id.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                    messages: Vec::new(),
                },
            );
            id
        } else if self.conversations.contains_key(requested_id) {
            requested_id.into()
        } else {
            return error_response(404, "conversation_not_found", "Conversation was not found.");
        };
        let user = Message {
            role: "user".into(),
            content: content.clone(),
            ..Default::default()
        };
        self.append(&conversation_id, user.clone(), Some(metadata.clone()));
        let messages = self.conversations[&conversation_id]
            .messages
            .iter()
            .map(|stored| stored.message.clone())
            .collect();
        let correlation = metadata
            .get("correlation_id")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| id("corr"));
        let mut request_headers = headers.clone();
        request_headers.insert("X-Actor-ID".into(), Value::String(actor.clone()));
        request_headers.insert(
            "X-Actor-Permissions".into(),
            Value::String(permissions.clone()),
        );
        let request = EngineRequest {
            messages,
            metadata: EngineRequestMetadata {
                correlation_id: correlation.clone(),
                conversation_id: conversation_id.clone(),
                client_context: metadata.clone(),
                ..Default::default()
            },
            model: self.current_model.clone(),
            auth_context: serde_json::json!({
                "actor": {"id": actor, "type": "owner", "display_name": "Owner", "status": "active", "created": timestamp()},
                "authentication_id": id("authn"), "authenticated_at": timestamp(), "correlation_id": correlation,
            }),
        };
        let mut stream = self.engine.run(&request, &request_headers);
        let text: String = stream
            .iter()
            .filter(|event| event.event == "text-delta")
            .filter_map(|event| event.data.get("content").and_then(Value::as_str))
            .collect();
        if let Some(done) = stream.iter_mut().find(|event| event.event == "done") {
            done.data
                .entry("message_id")
                .or_insert_with(|| Value::String(id("msg")));
            done.data
                .entry("finish_reason")
                .or_insert_with(|| Value::String("stop".into()));
        }
        if !text.is_empty() {
            self.append(
                &conversation_id,
                Message {
                    role: "assistant".into(),
                    content: vec![serde_json::json!({"type":"text","text":text})],
                    ..Default::default()
                },
                None,
            );
        }
        for event in &mut stream {
            event
                .data
                .insert("_event_type".into(), Value::String(event.event.clone()));
        }
        let mut response = sse_response(stream);
        response
            .headers
            .insert("X-Conversation-ID".into(), Value::String(conversation_id));
        response
    }
    fn append(
        &mut self,
        conversation_id: &str,
        message: Message,
        metadata: Option<Map<String, Value>>,
    ) {
        let now = timestamp();
        let message_id = id("msg");
        let conversation = self
            .conversations
            .get_mut(conversation_id)
            .expect("conversation");
        conversation.messages.push(StoredMessage {
            id: message_id.clone(),
            message: message.clone(),
            created_at: now.clone(),
        });
        conversation.updated_at = now;
        self.memory.record(Entry {
            kind: "message".into(),
            id: message_id,
            conversation_id: conversation_id.into(),
            role: message.role,
            content: message.content,
            metadata,
        });
    }
    fn list_conversations(&self) -> Response {
        let items: Vec<_> = self.conversations.values().map(|conversation| serde_json::json!({"id":conversation.id,"created_at":conversation.created_at,"updated_at":conversation.updated_at,"message_count":conversation.messages.len()})).collect();
        json_response(
            200,
            serde_json::json!({"conversations":items,"total":items.len()}),
        )
    }
    fn get_conversation(&self, id: &str) -> Response {
        let Some(conversation) = self.conversations.get(id) else {
            return error_response(404, "conversation_not_found", "Conversation was not found.");
        };
        let messages: Vec<_> = conversation.messages.iter().map(|stored| serde_json::json!({"id":stored.id,"role":stored.message.role,"content":stored.message.content,"created_at":stored.created_at})).collect();
        json_response(
            200,
            serde_json::json!({"id":conversation.id,"created_at":conversation.created_at,"updated_at":conversation.updated_at,"messages":messages}),
        )
    }
}
fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn id(prefix: &str) -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{prefix}_{nonce}")
}
fn headers(content_type: &str) -> Map<String, Value> {
    let mut result = Map::new();
    result.insert("Content-Type".into(), Value::String(content_type.into()));
    result
}
fn json_response(status: u16, body: Value) -> Response {
    Response {
        status,
        headers: headers("application/json"),
        body,
        stream: Vec::new(),
    }
}
fn error_response(status: u16, code: &str, message: &str) -> Response {
    json_response(status, serde_json::json!({"code":code,"message":message}))
}
fn sse_response(stream: Vec<StreamEvent>) -> Response {
    let mut response = Response {
        status: 200,
        headers: headers("text/event-stream"),
        body: Value::Null,
        stream,
    };
    response
        .headers
        .insert("Cache-Control".into(), Value::String("no-cache".into()));
    response
        .headers
        .insert("Connection".into(), Value::String("keep-alive".into()));
    response
}
