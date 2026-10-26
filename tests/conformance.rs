use paa_rust_architecture_template::{
    adapters::{load_model_adapter, OpenAiCompatibleAdapter},
    agent_loop::AgentLoop,
    gateway::Gateway,
    types::ToolDefinition,
};
use serde_json::{Map, Value};

#[test]
fn public_request_rejects_internal_agent_loop_shape() {
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

#[test]
fn new_tool_protocol_label_round_trips_with_source_identity() {
    let definition: ToolDefinition = serde_json::from_value(serde_json::json!({
        "name":"read", "description":"Read", "parameters":{},
        "source":{"id":"service-1","kind":"future-protocol"},
        "visibility":"always-send", "extensions":{"future":"ignored"}
    }))
    .unwrap();
    assert_eq!(definition.source["id"], "service-1");
    assert_eq!(definition.source["kind"], "future-protocol");
    let output = serde_json::to_value(&definition).unwrap();
    assert_eq!(output["source"]["kind"], "future-protocol");
}
