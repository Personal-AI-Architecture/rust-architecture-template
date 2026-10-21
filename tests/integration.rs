use paa_rust_architecture_template::{
    adapters::{load_model_adapter, OpenAiCompatibleAdapter},
    engine::AgentLoop,
    gateway::Gateway,
    types::{CompletionRequest, Message, ModelAdapter},
};
use serde_json::{Map, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

#[test]
fn canonical_route_streams_and_persists() {
    let adapter = load_model_adapter("mock", OpenAiCompatibleAdapter::default()).unwrap();
    let mut gateway = Gateway::new(AgentLoop::new(adapter, Vec::new()), None);
    let mut headers = Map::new();
    headers.insert("X-Actor-ID".into(), Value::String("integration".into()));
    headers.insert(
        "X-Actor-Permissions".into(),
        Value::String("memory:read,memory:write".into()),
    );
    let response = gateway.handle(
        "POST",
        "/conversations/messages",
        Some(serde_json::json!({"content":"hello"})),
        &headers,
    );
    assert_eq!(response.status, 200);
    assert!(response.stream.iter().any(|event| event.event == "done"));
    let id = response.headers["X-Conversation-ID"].as_str().unwrap();
    let detail = gateway.handle("GET", &format!("/conversations/{id}"), None, &Map::new());
    assert_eq!(detail.body["messages"].as_array().unwrap().len(), 2);
}

#[test]
fn openai_compatible_adapter_normalizes_sse() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    let address = listener.local_addr().expect("provider address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("provider connection");
        let mut request = [0_u8; 4096];
        let size = stream.read(&mut request).expect("provider request");
        assert!(String::from_utf8_lossy(&request[..size]).contains("\"stream\":true"));
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).expect("provider response");
    });
    let adapter = OpenAiCompatibleAdapter {
        api_base_url: format!("http://{address}"),
        model: "test-model".into(),
        ..Default::default()
    };
    let chunks = adapter
        .stream(&CompletionRequest {
            messages: vec![Message {
                role: "user".into(),
                content: "hello".into(),
                ..Default::default()
            }],
            stream: true,
            ..Default::default()
        })
        .expect("normalized provider stream");
    assert_eq!(chunks.len(), 2);
    assert_eq!(
        chunks[0].delta.get("content").and_then(Value::as_str),
        Some("ok")
    );
    assert_eq!(
        chunks[1].delta.get("finish_reason").and_then(Value::as_str),
        Some("stop")
    );
    server.join().expect("provider server");
}
