use serde_json::Value;

#[derive(Clone, Debug)]
pub struct RuntimeConfiguration {
    pub memory_root: String,
    pub provider_adapter: String,
    pub auth_mode: String,
    pub tool_sources: Vec<String>,
}

impl Default for RuntimeConfiguration {
    fn default() -> Self {
        Self {
            memory_root: ".paa-memory".into(),
            provider_adapter: "mock".into(),
            auth_mode: "enforced".into(),
            tool_sources: vec!["memory".into()],
        }
    }
}

pub fn load_runtime_configuration(values: Option<&Value>) -> Result<RuntimeConfiguration, String> {
    let mut result = RuntimeConfiguration::default();
    let Some(Value::Object(values)) = values else {
        return Ok(result);
    };
    let source = values
        .get("default")
        .and_then(Value::as_object)
        .unwrap_or(values);
    if let Some(value) = source.get("memory_root").and_then(Value::as_str) {
        result.memory_root = value.into();
    }
    if let Some(value) = source.get("provider_adapter").and_then(Value::as_str) {
        result.provider_adapter = value.into();
    }
    if let Some(value) = source.get("auth_mode").and_then(Value::as_str) {
        result.auth_mode = value.into();
    }
    if let Some(value) = source.get("tool_sources") {
        let Some(items) = value.as_array() else {
            return Err("tool_sources must be an array of strings".into());
        };
        result.tool_sources = items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "tool_sources must be an array of strings".to_owned())
            })
            .collect::<Result<_, _>>()?;
    }
    Ok(result)
}
