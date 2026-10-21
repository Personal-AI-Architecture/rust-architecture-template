use crate::types::{EngineRequest, ModelAdapter, StreamEvent};
use serde_json::{Map, Value};

pub struct AgentLoop {
    pub adapter: Box<dyn ModelAdapter>,
    pub tools: Vec<crate::types::ToolDefinition>,
}
impl AgentLoop {
    pub fn new(adapter: Box<dyn ModelAdapter>, tools: Vec<crate::types::ToolDefinition>) -> Self {
        Self { adapter, tools }
    }
    pub fn run(&self, request: &EngineRequest, headers: &Map<String, Value>) -> Vec<StreamEvent> {
        if request.metadata.correlation_id.trim().is_empty() {
            return vec![error_event("provider_error", "Missing correlation_id.")];
        }
        if header(headers, "X-Actor-ID").is_empty()
            || header(headers, "X-Actor-Permissions").is_empty()
        {
            return vec![error_event(
                "provider_error",
                "Missing required actor headers.",
            )];
        }
        let completion = crate::types::CompletionRequest {
            messages: request.messages.clone(),
            tools: self.tools.clone(),
            stream: true,
            ..Default::default()
        };
        let chunks = match self.adapter.stream(&completion) {
            Ok(chunks) => chunks,
            Err(error) => return vec![error_event("provider_error", &error)],
        };
        chunks
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
                            d.insert("text".into(), Value::String(text.into()));
                            StreamEvent {
                                event: "text-delta".into(),
                                data: d,
                            }
                        }),
                    "tool-call" | "tool-result" | "error" => Some(StreamEvent {
                        event: chunk.chunk_type,
                        data,
                    }),
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
            .collect()
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
