use crate::{
    agent_loop::AgentLoop,
    memory::{Entry, Store},
    types::{AgentLoopRequest, AgentLoopRequestMetadata, Message, StreamEvent},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: String,
    pub message: Message,
    pub created_at: String,
    pub model: Option<String>,
    pub provider: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
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
    pub agent_loop: AgentLoop,
    pub memory: Store,
    pub current_model: String,
    pub current_provider: String,
    pub preferences_from_memory: bool,
    conversations: HashMap<String, Conversation>,
    persistence_path: Option<PathBuf>,
}

impl Gateway {
    pub fn new(agent_loop: AgentLoop, memory: Option<Store>) -> Self {
        Self {
            agent_loop,
            memory: memory.unwrap_or_else(|| Store::new(".paa-memory")),
            current_model: "mock".into(),
            current_provider: "local".into(),
            preferences_from_memory: false,
            conversations: HashMap::new(),
            persistence_path: None,
        }
    }
    pub fn enable_persistence(&mut self, root: &str) -> std::io::Result<()> {
        std::fs::create_dir_all(root)?;
        let path = PathBuf::from(root).join("conversations.json");
        if path.exists() {
            let bytes = std::fs::read(&path)?;
            self.conversations = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        }
        self.persistence_path = Some(path);
        Ok(())
    }
    fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.persistence_path else {
            return Ok(());
        };
        let bytes =
            serde_json::to_vec_pretty(&self.conversations).map_err(std::io::Error::other)?;
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, path)
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
            return self
                .list_conversations(path.split_once('?').map(|(_, query)| query).unwrap_or(""));
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
        if content.iter().any(|part| {
            let Some(fields) = part.as_object() else {
                return true;
            };
            let allowed: &[&str] = match part.get("type").and_then(Value::as_str) {
                Some("text") if part.get("text").is_some_and(Value::is_string) => {
                    &["type", "text", "extensions"]
                }
                Some("image" | "file") if part.get("url").is_some_and(Value::is_string) => {
                    &["type", "url", "name", "mime_type", "extensions"]
                }
                _ => return true,
            };
            fields.keys().any(|key| !allowed.contains(&key.as_str()))
                || fields
                    .get("extensions")
                    .is_some_and(|value| !value.is_object())
        }) {
            return error_response(400, "invalid_request", "Invalid content part.");
        }
        let actor = crate::agent_loop::header(headers, "X-Actor-ID");
        let permissions = crate::agent_loop::header(headers, "X-Actor-Permissions");
        if actor.is_empty() || permissions.is_empty() {
            return error_response(
                401,
                "missing_actor_headers",
                "Missing required actor headers.",
            );
        }
        if self.preferences_from_memory {
            match self.memory.read_preferences() {
                Ok(Some(preference)) => {
                    self.current_model = preference.model;
                    self.current_provider = preference.active_provider_profile;
                }
                _ => {
                    return error_response(
                        503,
                        "selection_unavailable",
                        "Current model and provider preferences are unavailable.",
                    )
                }
            }
        }
        let Some(limit) = crate::types::context::limit_for(
            &self.agent_loop.context_limits,
            &self.current_provider,
            &self.current_model,
        ) else {
            return error_response(
                503,
                "context_limit_unknown",
                "No safe context limit for selected provider and model.",
            );
        };
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
        if self.save().is_err() {
            return error_response(500, "storage_error", "Conversation could not be saved.");
        }
        let messages: Vec<Message> = self.conversations[&conversation_id]
            .messages
            .iter()
            .map(|stored| stored.message.clone())
            .collect();
        let Some((selected, omitted)) =
            crate::types::context::select_complete_turns(&messages, &self.agent_loop.tools, limit)
        else {
            return error_response(
                422,
                "context_overflow",
                "Current message exceeds the selected model context window.",
            );
        };
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
        let request = AgentLoopRequest {
            messages: selected,
            metadata: AgentLoopRequestMetadata {
                correlation_id: correlation.clone(),
                conversation_id: conversation_id.clone(),
                client_context: metadata.clone(),
                ..Default::default()
            },
            model: self.current_model.clone(),
            provider: self.current_provider.clone(),
            context_summary: None,
            auth_context: serde_json::json!({
                "actor": {"id": actor, "type": "owner", "display_name": "Owner", "status": "active", "created": timestamp()},
                "authentication_id": id("authn"), "authenticated_at": timestamp(), "correlation_id": correlation,
            }),
        };
        let mut stream = self.agent_loop.run(&request, &request_headers);
        if omitted > 0 {
            stream.insert(0, StreamEvent { event: "context_notice".into(), data: Map::from_iter([
                ("action".into(), Value::String("omitted".into())),
                ("omitted_turn_count".into(), serde_json::json!(omitted)),
                ("message".into(), Value::String("Older conversation turns were omitted from this reply's working context.".into())),
            ]) });
        }
        let text: String = stream
            .iter()
            .filter(|event| event.event == "text-delta")
            .filter_map(|event| event.data.get("content").and_then(Value::as_str))
            .collect();
        if let Some(done) = stream.iter_mut().find(|event| event.event == "done") {
            done.data
                .entry("finish_reason")
                .or_insert_with(|| Value::String("stop".into()));
        }
        if stream.iter().any(|event| event.event == "done") {
            let provenance = Map::from_iter([
                ("model".into(), Value::String(self.current_model.clone())),
                (
                    "provider".into(),
                    Value::String(self.current_provider.clone()),
                ),
            ]);
            let message_id = self.append(
                &conversation_id,
                Message {
                    role: "assistant".into(),
                    content: vec![serde_json::json!({"type":"text","text":text})],
                    ..Default::default()
                },
                Some(provenance),
            );
            if let Some(done) = stream.iter_mut().find(|event| event.event == "done") {
                done.data
                    .insert("message_id".into(), Value::String(message_id));
            }
        }
        if self.save().is_err() {
            return error_response(500, "storage_error", "Conversation could not be saved.");
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
    ) -> String {
        let now = timestamp();
        let message_id = id("msg");
        let conversation = self
            .conversations
            .get_mut(conversation_id)
            .expect("conversation");
        let model = if message.role == "assistant" {
            metadata
                .as_ref()
                .and_then(|value| value.get("model"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        } else {
            None
        };
        let provider = if message.role == "assistant" {
            metadata
                .as_ref()
                .and_then(|value| value.get("provider"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        } else {
            None
        };
        if message.role == "assistant" {
            assert!(
                model.is_some() && provider.is_some(),
                "assistant reply requires model and provider"
            );
        }
        conversation.messages.push(StoredMessage {
            id: message_id.clone(),
            message: message.clone(),
            created_at: now.clone(),
            model,
            provider,
        });
        conversation.updated_at = now;
        self.memory.record(Entry {
            kind: "message".into(),
            id: message_id.clone(),
            conversation_id: conversation_id.into(),
            role: message.role,
            content: message.content,
            metadata,
        });
        message_id
    }
    fn list_conversations(&self, query: &str) -> Response {
        let params = parse_query(query);
        let limit = match params.get("limit") {
            Some(value) => value
                .parse::<usize>()
                .ok()
                .filter(|value| (1..=100).contains(value)),
            None => Some(20),
        };
        let offset = match params.get("offset") {
            Some(value) => value.parse::<usize>().ok(),
            None => Some(0),
        };
        let (Some(limit), Some(offset)) = (limit, offset) else {
            return error_response(400, "invalid_request", "Invalid pagination.");
        };
        let after = match params.get("from") {
            Some(value) => match DateTime::parse_from_rfc3339(value) {
                Ok(value) => Some(value.with_timezone(&Utc)),
                Err(_) => return error_response(400, "invalid_request", "Invalid from timestamp."),
            },
            None => None,
        };
        let before = match params.get("to") {
            Some(value) => match DateTime::parse_from_rfc3339(value) {
                Ok(value) => Some(value.with_timezone(&Utc)),
                Err(_) => return error_response(400, "invalid_request", "Invalid to timestamp."),
            },
            None => None,
        };
        let search = params
            .get("search")
            .map(|value| value.to_lowercase())
            .unwrap_or_default();
        let mut matches: Vec<_> = self
            .conversations
            .values()
            .filter(|conversation| {
                let created = DateTime::parse_from_rfc3339(&conversation.created_at)
                    .ok()
                    .map(|value| value.with_timezone(&Utc));
                if after.is_some_and(|bound| created.map_or(true, |date| date < bound)) {
                    return false;
                }
                if before.is_some_and(|bound| created.map_or(true, |date| date > bound)) {
                    return false;
                }
                search.is_empty()
                    || conversation.messages.iter().any(|message| {
                        serde_json::to_string(&message.message.content)
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(&search)
                    })
            })
            .collect();
        matches.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let total = matches.len();
        let items: Vec<_> = matches.into_iter().skip(offset).take(limit).map(|conversation| serde_json::json!({"id":conversation.id,"created_at":conversation.created_at,"updated_at":conversation.updated_at,"message_count":conversation.messages.len()})).collect();
        json_response(
            200,
            serde_json::json!({"conversations":items,"total":total,"limit":limit,"offset":offset}),
        )
    }
    fn get_conversation(&self, id: &str) -> Response {
        let Some(conversation) = self.conversations.get(id) else {
            return error_response(404, "conversation_not_found", "Conversation was not found.");
        };
        let messages: Vec<_> = conversation.messages.iter().map(|stored| {
            let mut item = serde_json::json!({"id":stored.id,"role":stored.message.role,"content":stored.message.content,"created_at":stored.created_at});
            if stored.message.role == "assistant" {
                item["model"] = serde_json::json!(stored.model);
                item["provider"] = serde_json::json!(stored.provider);
            }
            item
        }).collect();
        json_response(
            200,
            serde_json::json!({"id":conversation.id,"created_at":conversation.created_at,"updated_at":conversation.updated_at,"messages":messages}),
        )
    }
}

fn parse_query(query: &str) -> HashMap<String, String> {
    fn decode(value: &str) -> String {
        let mut bytes = Vec::new();
        let raw = value.as_bytes();
        let mut index = 0;
        while index < raw.len() {
            if raw[index] == b'%' && index + 2 < raw.len() {
                if let Ok(decoded) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                    bytes.push(decoded);
                    index += 3;
                    continue;
                }
            }
            bytes.push(if raw[index] == b'+' { b' ' } else { raw[index] });
            index += 1;
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .map(|(key, value)| (decode(key), decode(value)))
        .collect()
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
