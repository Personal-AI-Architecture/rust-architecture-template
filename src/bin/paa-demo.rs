use paa_rust_architecture_template::{
    adapters::{load_model_adapter, OpenAiCompatibleAdapter},
    engine::AgentLoop,
    gateway::Gateway,
};
use serde_json::{Map, Value};

fn main() {
    let adapter =
        load_model_adapter("mock", OpenAiCompatibleAdapter::default()).expect("mock adapter");
    let mut gateway = Gateway::new(AgentLoop::new(adapter, Vec::new()), None);
    let mut headers = Map::new();
    headers.insert("X-Actor-ID".into(), Value::String("demo-user".into()));
    headers.insert(
        "X-Actor-Permissions".into(),
        Value::String("memory:read,memory:write".into()),
    );
    let response = gateway.handle(
        "POST",
        "/conversations/messages",
        Some(serde_json::json!({"content":"Explain what PAA is."})),
        &headers,
    );
    println!("PAA Rust template demo\n======================");
    println!("Request: POST /conversations/messages");
    println!(
        "Conversation: {}",
        response
            .headers
            .get("X-Conversation-ID")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
    );
    println!("Stream:");
    for event in response.stream {
        println!(
            "  {}: {}",
            event.event,
            serde_json::to_string(&event.data).unwrap_or_default()
        );
    }
    println!(
        "\nPAA boundary demonstrated: Gateway -> Agent Loop -> Model Adapter -> stream -> memory."
    );
}
