use crate::types::{CompletionRequest, Message, ModelAdapter, ModelStreamChunk};
use reqwest::blocking::Client;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

#[derive(Clone, Debug, Default)]
pub struct ProviderProfile {
    pub base_url: String,
    pub api_key_env: Option<String>,
    pub model_context_limits: HashMap<String, usize>,
}

#[derive(Clone, Debug, Default)]
pub struct OpenAiCompatibleAdapter {
    pub api_base_url: String,
    pub api_key: String,
    pub client: Option<Client>,
    pub provider_profiles: HashMap<String, ProviderProfile>,
}

impl ModelAdapter for OpenAiCompatibleAdapter {
    fn name(&self) -> &str {
        "openai-compatible"
    }
    fn context_limits(&self) -> Option<crate::types::context::ContextLimits> {
        Some(
            self.provider_profiles
                .iter()
                .map(|(name, profile)| (name.clone(), profile.model_context_limits.clone()))
                .collect(),
        )
    }
    fn stream(&self, request: &CompletionRequest) -> Result<Vec<ModelStreamChunk>, String> {
        if request.model.trim().is_empty() || request.provider.trim().is_empty() {
            return Err("Model API request requires model and provider.".into());
        }
        let (base_url, api_key) = if self.provider_profiles.is_empty() {
            if request.provider != "local" {
                return Err("Unknown selected provider profile.".into());
            }
            (self.api_base_url.as_str(), self.api_key.clone())
        } else {
            let profile = self
                .provider_profiles
                .get(&request.provider)
                .ok_or("Unknown selected provider profile.")?;
            let key = match &profile.api_key_env {
                Some(name) => std::env::var(name)
                    .map_err(|_| "Selected provider profile secret is unavailable.")?,
                None => String::new(),
            };
            (profile.base_url.as_str(), key)
        };
        if base_url.trim().is_empty() {
            return Err("Selected provider profile has no base URL.".into());
        }
        let client = self.client.clone().unwrap_or_default();
        let model = request.model.clone();
        let payload = json!({
            "model": model,
            "messages": request.messages.iter().map(provider_message).collect::<Vec<_>>(),
            "stream": true
        });
        let endpoint = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let mut call = client.post(endpoint).json(&payload);
        if !api_key.is_empty() {
            call = call.bearer_auth(&api_key);
        }
        let response = call.send().map_err(|error| error.to_string())?;
        if !response.status().is_success() {
            let body = response.text().unwrap_or_default();
            let payload: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            return Ok(vec![ModelStreamChunk {
                chunk_type: "error".into(),
                delta: normalized_provider_failure(&payload),
            }]);
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

fn normalized_provider_failure(payload: &Value) -> Map<String, Value> {
    let failure = payload.get("error").unwrap_or(payload);
    let code = failure["code"]
        .as_str()
        .or_else(|| failure["type"].as_str());
    let mut result = Map::new();
    match code {
        Some("model_not_found" | "model_unavailable") => {
            result.insert("code".into(), Value::String("model_unavailable".into()));
        }
        Some("unsupported_capability" | "capability_unsupported") => {
            let capability = failure["capability"]
                .as_str()
                .or_else(|| failure["param"].as_str())
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
                result.insert(
                    "code".into(),
                    Value::String("capability_unsupported".into()),
                );
                result.insert("capability".into(), Value::String(capability.into()));
            }
        }
        _ => {}
    }
    if !result.contains_key("code") {
        result.insert("code".into(), Value::String("provider_error".into()));
    }
    result
}

fn provider_message(message: &Message) -> Value {
    let mut value = json!({"role": message.role, "content": message.content});
    if !message.tool_call_id.is_empty() {
        value["tool_call_id"] = Value::String(message.tool_call_id.clone());
    }
    value
}
