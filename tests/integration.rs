use paa_rust_architecture_template::{
    adapters::{load_model_adapter, OpenAiCompatibleAdapter, ProviderProfile},
    engine::AgentLoop,
    gateway::Gateway,
    types::{
        context, CompletionRequest, EngineRequest, EngineRequestMetadata, Message, ModelAdapter,
        ModelStreamChunk,
    },
};
use serde_json::{Map, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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
        Some(serde_json::json!({"content":[{"type":"text","text":"hello"}]})),
        &headers,
    );
    assert_eq!(response.status, 200);
    assert!(response.stream.iter().any(|event| event.event == "done"));
    let id = response.headers["X-Conversation-ID"].as_str().unwrap();
    let detail = gateway.handle("GET", &format!("/conversations/{id}"), None, &Map::new());
    assert_eq!(detail.body["messages"].as_array().unwrap().len(), 2);
    assert!(detail.body["messages"][0]["content"].is_array());
    assert!(detail.body["messages"][1]["content"].is_array());
}

#[derive(Clone)]
struct FailingAdapter {
    code: &'static str,
    capability: Option<&'static str>,
    calls: Arc<Mutex<Vec<(String, String)>>>,
}
impl ModelAdapter for FailingAdapter {
    fn name(&self) -> &str {
        "failing"
    }
    fn stream(&self, request: &CompletionRequest) -> Result<Vec<ModelStreamChunk>, String> {
        self.calls
            .lock()
            .unwrap()
            .push((request.model.clone(), request.provider.clone()));
        Ok(vec![ModelStreamChunk { chunk_type: "error".into(), delta: serde_json::json!({"code":self.code,"capability":self.capability,"message":"secret=/tmp/key"}).as_object().unwrap().clone() }])
    }
}

#[test]
fn model_error_categories_reach_gateway_without_fallback_or_raw_text() {
    for (code, capability) in [
        ("model_unavailable", None),
        ("capability_unsupported", Some("image_input")),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let adapter = FailingAdapter {
            code,
            capability,
            calls: calls.clone(),
        };
        let mut gateway = Gateway::new(AgentLoop::new(Box::new(adapter), Vec::new()), None);
        let mut headers = Map::new();
        headers.insert("X-Actor-ID".into(), Value::String("owner".into()));
        headers.insert(
            "X-Actor-Permissions".into(),
            Value::String("memory:read,memory:write".into()),
        );
        let response = gateway.handle(
            "POST",
            "/conversations/messages",
            Some(serde_json::json!({"content":[{"type":"text","text":"hello"}]})),
            &headers,
        );
        assert_eq!(response.status, 200);
        assert_eq!(response.stream.len(), 1);
        assert_eq!(response.stream[0].data["code"], code);
        if let Some(expected) = capability {
            assert_eq!(response.stream[0].data["capability"], expected);
        }
        assert!(!format!("{:?}", response.stream).contains("secret"));
        assert_eq!(
            *calls.lock().unwrap(),
            vec![("mock".into(), "local".into())]
        );
    }
}

#[test]
fn extension_parts_are_accepted_and_unknown_core_fields_rejected() {
    let adapter = load_model_adapter("mock", OpenAiCompatibleAdapter::default()).unwrap();
    let mut gateway = Gateway::new(AgentLoop::new(adapter, Vec::new()), None);
    let mut headers = Map::new();
    headers.insert("X-Actor-ID".into(), Value::String("owner".into()));
    headers.insert(
        "X-Actor-Permissions".into(),
        Value::String("memory:read,memory:write".into()),
    );
    let valid = gateway.handle("POST", "/conversations/messages", Some(serde_json::json!({"content":[{"type":"text","text":"hello","extensions":{"future":"ignored"}}]})), &headers);
    assert_eq!(valid.status, 200);
    let invalid = gateway.handle("POST", "/conversations/messages", Some(serde_json::json!({"content":[{"type":"text","text":"hello","future":"outside extensions"}]})), &headers);
    assert_eq!(invalid.status, 400);
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
        ..Default::default()
    };
    let chunks = adapter
        .stream(&CompletionRequest {
            messages: vec![Message {
                role: "user".into(),
                content: vec![serde_json::json!({"type":"text","text":"hello"})],
                ..Default::default()
            }],
            stream: true,
            model: "test-model".into(),
            provider: "local".into(),
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

#[test]
fn openai_adapter_normalizes_explicit_model_errors() {
    for (provider_code, capability, expected) in [
        ("model_not_found", "", "model_unavailable"),
        (
            "unsupported_capability",
            "image_input",
            "capability_unsupported",
        ),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            let body = format!(
                r#"{{"error":{{"code":"{provider_code}","capability":"{capability}","message":"secret=/tmp/key"}}}}"#
            );
            write!(stream, "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let adapter = OpenAiCompatibleAdapter {
            api_base_url: format!("http://{address}"),
            ..Default::default()
        };
        let chunks = adapter
            .stream(&CompletionRequest {
                model: "m".into(),
                provider: "local".into(),
                stream: true,
                ..Default::default()
            })
            .unwrap();
        server.join().unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta["code"], expected);
        assert!(chunks[0].delta.get("message").is_none());
    }
}

fn provider_server(reply: &'static str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("provider listener");
    let address = listener.local_addr().expect("provider address");
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("provider connection");
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request).expect("provider request");
        let body = format!("data: {{\"choices\":[{{\"delta\":{{\"content\":\"{reply}\"}}}}]}}\n\n");
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).expect("provider response");
    });
    (format!("http://{address}"), handle)
}

#[test]
fn provider_profiles_route_each_request_without_fallback() {
    let (alpha_url, alpha) = provider_server("alpha");
    let (beta_url, beta) = provider_server("beta");
    let adapter = OpenAiCompatibleAdapter {
        provider_profiles: [
            (
                "alpha".into(),
                ProviderProfile {
                    base_url: alpha_url,
                    model_context_limits: [("m".into(), 8192)].into(),
                    ..Default::default()
                },
            ),
            (
                "beta".into(),
                ProviderProfile {
                    base_url: beta_url,
                    model_context_limits: [("m".into(), 4096)].into(),
                    ..Default::default()
                },
            ),
        ]
        .into(),
        client: Some(
            reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(3))
                .build()
                .unwrap(),
        ),
        ..Default::default()
    };
    assert_eq!(adapter.context_limits().unwrap()["beta"]["m"], 4096);
    for provider in ["alpha", "beta"] {
        let chunks = adapter
            .stream(&CompletionRequest {
                messages: vec![Message {
                    role: "user".into(),
                    content: vec![serde_json::json!({"type":"text","text":"hello"})],
                    ..Default::default()
                }],
                stream: true,
                model: "m".into(),
                provider: provider.into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(chunks[0].delta["content"], provider);
    }
    assert!(adapter
        .stream(&CompletionRequest {
            model: "m".into(),
            provider: "missing".into(),
            ..Default::default()
        })
        .is_err());
    assert!(adapter
        .stream(&CompletionRequest {
            provider: "alpha".into(),
            ..Default::default()
        })
        .is_err());
    alpha.join().unwrap();
    beta.join().unwrap();
}

#[derive(Clone)]
struct RecordingAdapter(Arc<Mutex<Vec<CompletionRequest>>>);
impl ModelAdapter for RecordingAdapter {
    fn name(&self) -> &str {
        "recording"
    }
    fn stream(&self, request: &CompletionRequest) -> Result<Vec<ModelStreamChunk>, String> {
        self.0.lock().unwrap().push(request.clone());
        Ok(vec![
            ModelStreamChunk {
                chunk_type: "text-delta".into(),
                delta: Map::from_iter([("content".into(), Value::String("answer".into()))]),
            },
            ModelStreamChunk {
                chunk_type: "done".into(),
                delta: Map::from_iter([("finish_reason".into(), Value::String("stop".into()))]),
            },
        ])
    }
}

#[test]
fn derived_summary_is_labelled_and_ephemeral() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let loop_ = AgentLoop::new(Box::new(RecordingAdapter(recorded.clone())), Vec::new());
    let original = vec![Message {
        role: "user".into(),
        content: vec![serde_json::json!({"type":"text","text":"current"})],
        ..Default::default()
    }];
    let request = EngineRequest {
        messages: original.clone(),
        metadata: EngineRequestMetadata {
            correlation_id: "corr".into(),
            ..Default::default()
        },
        model: "mock".into(),
        provider: "local".into(),
        auth_context: serde_json::json!({"actor":{"id":"owner"},"correlation_id":"corr"}),
        context_summary: Some(
            serde_json::json!({"source":"gateway","method":"deterministic","omitted_turn_count":1,"text":"older fact"}),
        ),
    };
    loop_.run(&request, &Map::new());
    let calls = recorded.lock().unwrap();
    assert_eq!(original.len(), 1);
    assert_eq!(calls[0].messages.len(), 2);
    assert!(calls[0].messages[0].content[0]["text"]
        .as_str()
        .unwrap()
        .contains("Gateway-derived context"));
    assert_eq!(calls[0].messages[1].content[0]["text"], "current");
}

#[test]
fn auth_context_rejects_unknown_top_level_field() {
    let recorded = Arc::new(Mutex::new(Vec::new()));
    let loop_ = AgentLoop::new(Box::new(RecordingAdapter(recorded.clone())), Vec::new());
    let request = EngineRequest {
        messages: Vec::new(),
        metadata: EngineRequestMetadata {
            correlation_id: "corr".into(),
            ..Default::default()
        },
        model: "mock".into(),
        provider: "local".into(),
        auth_context: serde_json::json!({"actor":{"id":"owner"},"correlation_id":"corr","unexpected":"spoof"}),
        context_summary: None,
    };
    let events = loop_.run(&request, &Map::new());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["code"], "auth_error");
    assert!(recorded.lock().unwrap().is_empty());
}

fn owner_headers() -> Map<String, Value> {
    Map::from_iter([
        ("X-Actor-ID".into(), Value::String("owner".into())),
        ("X-Actor-Permissions".into(), Value::String("read".into())),
    ])
}

#[test]
fn provider_switch_reaches_model_and_saves_reply_provenance() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut loop_ = AgentLoop::new(Box::new(RecordingAdapter(requests.clone())), Vec::new());
    loop_.context_limits = [
        ("alpha".into(), [("m1".into(), 8192)].into()),
        ("beta".into(), [("m2".into(), 8192)].into()),
    ]
    .into();
    let mut gateway = Gateway::new(loop_, None);
    gateway.current_provider = "alpha".into();
    gateway.current_model = "m1".into();
    let body = serde_json::json!({"content":[{"type":"text","text":"hello"}]});
    let first = gateway.handle(
        "POST",
        "/conversations/messages",
        Some(body.clone()),
        &owner_headers(),
    );
    let id = first.headers["X-Conversation-ID"].as_str().unwrap();
    let path = format!("/conversations/{id}/messages");
    gateway.current_provider = "beta".into();
    gateway.current_model = "m2".into();
    let second = gateway.handle("POST", &path, Some(body), &owner_headers());
    assert_eq!((first.status, second.status), (200, 200));
    let recorded = requests.lock().unwrap();
    assert_eq!(
        (
            recorded[0].provider.as_str(),
            recorded[1].provider.as_str(),
            recorded[1].model.as_str()
        ),
        ("alpha", "beta", "m2")
    );
    let detail = gateway.handle("GET", &format!("/conversations/{id}"), None, &Map::new());
    assert_eq!(detail.body["messages"][1]["provider"], "alpha");
    assert_eq!(detail.body["messages"][3]["model"], "m2");
}

#[test]
fn context_limits_block_unknown_and_oversized_and_notice_omission() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mut loop_ = AgentLoop::new(Box::new(RecordingAdapter(requests.clone())), Vec::new());
    loop_.context_limits.clear();
    let mut gateway = Gateway::new(loop_, None);
    let body = serde_json::json!({"content":[{"type":"text","text":"hello"}]});
    let unknown = gateway.handle(
        "POST",
        "/conversations/messages",
        Some(body.clone()),
        &owner_headers(),
    );
    assert_eq!(
        (unknown.status, unknown.body["code"].as_str()),
        (503, Some("context_limit_unknown"))
    );
    gateway.engine.context_limits = context::default_limits();
    gateway
        .engine
        .context_limits
        .get_mut("local")
        .unwrap()
        .insert("mock".into(), 500);
    let huge = gateway.handle(
        "POST",
        "/conversations/messages",
        Some(serde_json::json!({"content":[{"type":"text","text":"x".repeat(1000)}]})),
        &owner_headers(),
    );
    assert_eq!(
        (huge.status, huge.body["code"].as_str()),
        (422, Some("context_overflow"))
    );
    assert!(requests.lock().unwrap().is_empty());
    let one = Message {
        role: "user".into(),
        content: vec![serde_json::json!({"type":"text","text":"hello"})],
        ..Default::default()
    };
    let one_cost = context::context_cost(&[one], &[]);
    gateway
        .engine
        .context_limits
        .get_mut("local")
        .unwrap()
        .insert("mock".into(), one_cost + 8);
    let first = gateway.handle(
        "POST",
        "/conversations/messages",
        Some(body.clone()),
        &owner_headers(),
    );
    let id = first.headers["X-Conversation-ID"].as_str().unwrap();
    let second = gateway.handle(
        "POST",
        &format!("/conversations/{id}/messages"),
        Some(body),
        &owner_headers(),
    );
    assert_eq!(second.stream[0].event, "context_notice");
    assert_eq!(requests.lock().unwrap().last().unwrap().messages.len(), 1);
    let detail = gateway.handle("GET", &format!("/conversations/{id}"), None, &Map::new());
    assert_eq!(detail.body["messages"].as_array().unwrap().len(), 4);
}
