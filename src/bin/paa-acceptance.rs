use paa_rust_architecture_template::{
    adapters::{load_model_adapter, MockAdapter, OpenAiCompatibleAdapter},
    agent_loop::AgentLoop,
    checks, config,
    gateway::Gateway,
    memory::Store,
};
use serde_json::{Map, Value};

fn main() {
    if let Err(error) = run() {
        eprintln!("Acceptance check failed: {error}");
        std::process::exit(1);
    }
    println!("Acceptance check passed.");
}
fn run() -> Result<(), String> {
    let runtime = config::load_runtime_configuration(Some(
        &serde_json::json!({"default":{"memory_root":".paa-memory","provider_adapter":"mock","auth_mode":"enforced","tool_sources":["memory"]}}),
    ))?;
    let adapter = load_model_adapter(
        &runtime.provider_adapter,
        OpenAiCompatibleAdapter {
            api_base_url: "http://provider.local/v1".into(),
            ..Default::default()
        },
    )?;
    if adapter.name() != "mock" {
        return Err("mock adapter selection failed".into());
    }
    let openai = load_model_adapter("openai-compatible", OpenAiCompatibleAdapter::default())?;
    if openai.name() != "openai-compatible" {
        return Err("OpenAI-compatible adapter selection failed".into());
    }
    if load_model_adapter("unknown", OpenAiCompatibleAdapter::default()).is_ok() {
        return Err("unknown adapter should be rejected".into());
    }
    let memory = Store::new(runtime.memory_root);
    let mut gateway = Gateway::new(
        AgentLoop::new(
            Box::new(MockAdapter {
                response: "acceptance-ok".into(),
            }),
            Vec::new(),
        ),
        Some(memory.clone()),
    );
    let mut headers = Map::new();
    headers.insert(
        "x-actor-id".into(),
        Value::String("actor-acceptance".into()),
    );
    headers.insert(
        "x-actor-permissions".into(),
        Value::String("memory:read,memory:write".into()),
    );
    let response = gateway.handle(
        "POST",
        "/conversations/messages",
        Some(serde_json::json!({"content":[{"type":"text","text":"Hello"}]})),
        &headers,
    );
    if response.status != 200 || response.stream.len() < 2 {
        return Err("canonical gateway route did not stream".into());
    }
    let id = response
        .headers
        .get("X-Conversation-ID")
        .and_then(Value::as_str)
        .ok_or("conversation id missing")?
        .to_owned();
    let list = gateway.handle("GET", "/conversations", None, &Map::new());
    if list.body["total"] != 1 {
        return Err("conversation list did not contain created conversation".into());
    }
    let detail = gateway.handle("GET", &format!("/conversations/{id}"), None, &Map::new());
    if detail.body["messages"]
        .as_array()
        .map_or(true, |messages| messages.len() != 2)
    {
        return Err("conversation detail did not persist user and assistant messages".into());
    }
    if memory.export()["history"]
        .as_array()
        .map_or(true, |history| history.len() < 2)
    {
        return Err("memory history was not exported".into());
    }
    if !checks::check_contracts(std::path::Path::new("."), None).is_empty()
        || !checks::check_boundaries(std::path::Path::new(".")).is_empty()
        || !checks::check_lock_in(std::path::Path::new(".")).is_empty()
    {
        return Err("native PAA checks failed".into());
    }
    Ok(())
}
