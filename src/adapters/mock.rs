use crate::types::{CompletionRequest, ModelAdapter, ModelStreamChunk};
use serde_json::{Map, Value};

#[derive(Clone, Debug)]
pub struct MockAdapter {
    pub response: String,
}
impl Default for MockAdapter {
    fn default() -> Self {
        Self {
            response: "mock response".into(),
        }
    }
}
impl ModelAdapter for MockAdapter {
    fn name(&self) -> &str {
        "mock"
    }
    fn stream(&self, _request: &CompletionRequest) -> Result<Vec<ModelStreamChunk>, String> {
        let mut text = Map::new();
        text.insert("content".into(), Value::String(self.response.clone()));
        let mut done = Map::new();
        done.insert("finish_reason".into(), Value::String("stop".into()));
        Ok(vec![
            ModelStreamChunk {
                chunk_type: "text-delta".into(),
                delta: text,
            },
            ModelStreamChunk {
                chunk_type: "done".into(),
                delta: done,
            },
        ])
    }
}
