use paa_rust_architecture_template::{
    adapters::{load_model_adapter, OpenAiCompatibleAdapter},
    engine::AgentLoop,
    gateway::Gateway,
};
use serde_json::{Map, Value};

#[test]
fn public_request_rejects_internal_engine_shape() {
    let adapter = load_model_adapter("mock", OpenAiCompatibleAdapter::default()).unwrap();
    let mut gateway = Gateway::new(AgentLoop::new(adapter, Vec::new()), None);
    let mut headers = Map::new();
    headers.insert("X-Actor-ID".into(), Value::String("actor".into()));
    headers.insert(
        "X-Actor-Permissions".into(),
        Value::String("memory:read".into()),
    );
    let response = gateway.handle(
        "POST",
        "/conversations/messages",
        Some(serde_json::json!({"messages":[],"content":"hello"})),
        &headers,
    );
    assert_eq!(response.status, 400);
}

#[test]
fn actor_headers_are_enforced() {
    let adapter = load_model_adapter("mock", OpenAiCompatibleAdapter::default()).unwrap();
    let mut gateway = Gateway::new(AgentLoop::new(adapter, Vec::new()), None);
    let response = gateway.handle(
        "POST",
        "/conversations/messages",
        Some(serde_json::json!({"content":[{"type":"text","text":"hello"}]})),
        &Map::new(),
    );
    assert_eq!(response.status, 401);
}

#[test]
fn old_string_content_is_rejected() {
    let adapter = load_model_adapter("mock", OpenAiCompatibleAdapter::default()).unwrap();
    let mut gateway = Gateway::new(AgentLoop::new(adapter, Vec::new()), None);
    let mut headers = Map::new();
    headers.insert("X-Actor-ID".into(), Value::String("actor".into()));
    headers.insert(
        "X-Actor-Permissions".into(),
        Value::String("memory:read".into()),
    );
    let response = gateway.handle(
        "POST",
        "/conversations/messages",
        Some(serde_json::json!({"content":"hello"})),
        &headers,
    );
    assert_eq!(response.status, 400);
}
