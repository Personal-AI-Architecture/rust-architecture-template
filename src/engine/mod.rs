use crate::types::{EngineRequest, ModelAdapter, StreamEvent};
use serde_json::{Map, Value};

pub struct AgentLoop {
    pub adapter: Box<dyn ModelAdapter>,
    pub tools: Vec<crate::types::ToolDefinition>,
    pub context_limits: crate::types::context::ContextLimits,
}
impl AgentLoop {
    pub fn new(adapter: Box<dyn ModelAdapter>, tools: Vec<crate::types::ToolDefinition>) -> Self {
        let context_limits = adapter
            .context_limits()
            .unwrap_or_else(crate::types::context::default_limits);
        Self {
            adapter,
            tools,
            context_limits,
        }
    }
    pub fn run(&self, request: &EngineRequest, _headers: &Map<String, Value>) -> Vec<StreamEvent> {
        if request.auth_context.as_object().is_some_and(|context| {
            context.keys().any(|key| {
                !matches!(
                    key.as_str(),
                    "actor"
                        | "authentication_id"
                        | "authenticated_at"
                        | "expires_at"
                        | "correlation_id"
                )
            })
        }) {
            return vec![error_event(
                "auth_error",
                "Authentication context was rejected.",
            )];
        }
        if request.metadata.correlation_id.trim().is_empty() {
            return vec![error_event("provider_error", "Missing correlation_id.")];
        }
        if request.auth_context["actor"]["id"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
            || request.auth_context["correlation_id"].as_str()
                != Some(request.metadata.correlation_id.as_str())
        {
            return vec![error_event(
                "provider_error",
                "Missing authenticated actor context.",
            )];
        }
        if request.model.is_empty() {
            return vec![error_event("invalid_request", "Missing selected model.")];
        }
        if request.provider.is_empty() {
            return vec![error_event("invalid_request", "Missing selected provider.")];
        }
        let Some(limit) = crate::types::context::limit_for(
            &self.context_limits,
            &request.provider,
            &request.model,
        ) else {
            return vec![error_event(
                "context_limit_unknown",
                "No safe context limit for selected provider and model.",
            )];
        };
        let mut messages = request.messages.clone();
        if let Some(summary) = &request.context_summary {
            if summary["source"] != "gateway"
                || summary["method"] != "deterministic"
                || !summary["text"].is_string()
            {
                return vec![error_event("invalid_request", "Invalid context summary.")];
            }
            messages.insert(0, crate::types::Message { role: "user".into(), content: vec![serde_json::json!({"type":"text","text":format!("Gateway-derived context from omitted turns (not a new owner request): {}", summary["text"].as_str().unwrap_or_default())})], ..Default::default() });
        }
        if crate::types::context::context_cost(&messages, &self.tools) > limit {
            return vec![error_event(
                "context_overflow",
                "Selected model context window exceeded.",
            )];
        }
        let completion = crate::types::CompletionRequest {
            messages,
            tools: self.tools.clone(),
            stream: true,
            model: request.model.clone(),
            provider: request.provider.clone(),
            ..Default::default()
        };
        let chunks = match self.adapter.stream(&completion) {
            Ok(chunks) => chunks,
            Err(_error) => return vec![error_event("provider_error", "Model provider failed.")],
        };
        let mut events: Vec<StreamEvent> = chunks
            .into_iter()
            .filter_map(|chunk| {
                let mut data = chunk.delta;
                match chunk.chunk_type.as_str() {
                    "text-delta" => data
                        .get("content")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty())
                        .map(|text| {
                            let mut d = Map::new();
                            d.insert("content".into(), Value::String(text.into()));
                            StreamEvent {
                                event: "text-delta".into(),
                                data: d,
                            }
                        }),
                    "tool-call" | "tool-result" => Some(StreamEvent {
                        event: chunk.chunk_type,
                        data,
                    }),
                    "error" => Some(safe_model_error(&data)),
                    "done" => {
                        if !data.contains_key("finish_reason") {
                            data.insert("finish_reason".into(), Value::String("stop".into()));
                        }
                        Some(StreamEvent {
                            event: "done".into(),
                            data,
                        })
                    }
                    _ => None,
                }
            })
            .collect();
        if let Some(index) = events.iter().position(|event| event.event == "error") {
            events.truncate(index + 1);
        }
        events
    }
}
fn safe_model_error(data: &Map<String, Value>) -> StreamEvent {
    match data.get("code").and_then(Value::as_str) {
        Some("model_unavailable") => error_event(
            "model_unavailable",
            "The selected model is unavailable through the selected provider.",
        ),
        Some("capability_unsupported") => {
            let capability = data
                .get("capability")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let valid = capability.len() <= 64
                && capability
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                && capability
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
            if valid {
                let mut event = error_event(
                    "capability_unsupported",
                    "The selected model does not support the requested capability.",
                );
                event
                    .data
                    .insert("capability".into(), Value::String(capability.into()));
                return event;
            }
            error_event("provider_error", "Model provider failed.")
        }
        _ => error_event("provider_error", "Model provider failed."),
    }
}
fn error_event(code: &str, message: &str) -> StreamEvent {
    let mut data = Map::new();
    data.insert("code".into(), Value::String(code.into()));
    data.insert("message".into(), Value::String(message.into()));
    StreamEvent {
        event: "error".into(),
        data,
    }
}
pub fn header(headers: &Map<String, Value>, name: &str) -> String {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .and_then(|(_, value)| value.as_str())
        .unwrap_or_default()
        .trim()
        .into()
}
