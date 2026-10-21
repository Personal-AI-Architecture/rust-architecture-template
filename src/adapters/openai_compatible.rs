use crate::types::{CompletionRequest, Message, ModelAdapter, ModelStreamChunk};
use reqwest::blocking::Client;
use serde_json::{json, Value};

#[derive(Clone, Debug, Default)]
pub struct OpenAiCompatibleAdapter {
    pub api_base_url: String,
    pub api_key: String,
    pub model: String,
    pub client: Option<Client>,
}

impl ModelAdapter for OpenAiCompatibleAdapter {
    fn name(&self) -> &str {
        "openai-compatible"
    }
    fn stream(&self, request: &CompletionRequest) -> Result<Vec<ModelStreamChunk>, String> {
        if self.api_base_url.trim().is_empty() {
            return Err("OpenAI-compatible adapter requires api_base_url.".into());
        }
        let client = self.client.clone().unwrap_or_default();
        let model = if request.model.is_empty() {
            self.model.clone()
        } else {
            request.model.clone()
        };
        let payload = json!({
            "model": model,
            "messages": request.messages.iter().map(provider_message).collect::<Vec<_>>(),
            "stream": true
        });
        let endpoint = format!(
            "{}/chat/completions",
            self.api_base_url.trim_end_matches('/')
        );
        let mut call = client.post(endpoint).json(&payload);
        if !self.api_key.is_empty() {
            call = call.bearer_auth(&self.api_key);
        }
        let response = call.send().map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            return Err(format!("provider returned HTTP {}", response.status()));
        }
        let mut chunks = Vec::new();
        for line in response.text().map_err(|error| error.to_string())?.lines() {
            let data = line
                .trim()
                .strip_prefix("data:")
                .map(str::trim)
                .unwrap_or_default();
            if data.is_empty() {
                continue;
            }
            if data == "[DONE]" {
                break;
            }
            let value: Value = serde_json::from_str(data).map_err(|error| error.to_string())?;
            let Some(choice) = value
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
            else {
                continue;
            };
            if let Some(delta) = choice.get("delta").and_then(Value::as_object) {
                if !delta.is_empty() {
                    chunks.push(ModelStreamChunk {
                        chunk_type: "text-delta".into(),
                        delta: delta.clone(),
                    });
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                if !reason.is_empty() {
                    chunks.push(ModelStreamChunk {
                        chunk_type: "done".into(),
                        delta: serde_json::Map::from_iter([(
                            String::from("finish_reason"),
                            Value::String(reason.into()),
                        )]),
                    });
                }
            }
        }
        Ok(chunks)
    }
}

fn provider_message(message: &Message) -> Value {
    let mut value = json!({"role": message.role, "content": message.content});
    if !message.tool_call_id.is_empty() {
        value["tool_call_id"] = Value::String(message.tool_call_id.clone());
    }
    value
}
