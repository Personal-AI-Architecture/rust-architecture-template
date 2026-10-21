mod mock;
mod openai_compatible;

pub use mock::MockAdapter;
pub use openai_compatible::OpenAiCompatibleAdapter;

use crate::types::ModelAdapter;

pub fn load_model_adapter(
    provider: &str,
    openai: OpenAiCompatibleAdapter,
) -> Result<Box<dyn ModelAdapter>, String> {
    match provider {
        "mock" | "local" => Ok(Box::new(MockAdapter::default())),
        "openai-compatible" => Ok(Box::new(openai)),
        other => Err(format!("unsupported provider adapter: {other}")),
    }
}
