use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub arguments: Map<String, Value>,
    #[serde(default)]
    pub invalid_arguments: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Vec<Value>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    #[serde(default)]
    pub tool_call_id: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub parameters: Map<String, Value>,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub mutates_state: bool,
    #[serde(default)]
    pub required_permissions: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EngineRequestMetadata {
    pub correlation_id: String,
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub trigger: String,
    #[serde(default)]
    pub client_context: Map<String, Value>,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub actor_permissions: Vec<String>,
    #[serde(default)]
    pub allowed_tool_sources: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EngineRequest {
    pub messages: Vec<Message>,
    pub metadata: EngineRequestMetadata,
    pub model: String,
    pub auth_context: Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    pub stream: bool,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub max_tokens: u32,
    #[serde(default)]
    pub temperature: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelStreamChunk {
    #[serde(rename = "type")]
    pub chunk_type: String,
    #[serde(default)]
    pub delta: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StreamEvent {
    pub event: String,
    #[serde(default)]
    pub data: Map<String, Value>,
}

pub trait ModelAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn stream(&self, request: &CompletionRequest) -> Result<Vec<ModelStreamChunk>, String>;
}
