use paa_rust_architecture_template::{
    adapters::{load_model_adapter, OpenAiCompatibleAdapter, ProviderProfile},
    engine::AgentLoop,
    gateway::{Gateway, Response},
};
use serde_json::{json, Map, Value};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
};

fn main() {
    let mut args = std::env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "127.0.0.1".into());
    let port = args.next().unwrap_or_else(|| "8000".into());
    let api_key = Arc::new(
        std::env::var("PAA_DEMO_API_KEY")
            .expect("set PAA_DEMO_API_KEY to a local test token before starting"),
    );
    assert!(!api_key.is_empty(), "PAA_DEMO_API_KEY must not be empty");
    assert!(
        std::env::var("RUNTIME_AUTH_MODE").map_or(true, |mode| mode == "enforced"),
        "HTTP starter requires enforced Auth"
    );
    let provider_adapter =
        std::env::var("RUNTIME_PROVIDER_ADAPTER").unwrap_or_else(|_| "mock".into());
    let mut openai = OpenAiCompatibleAdapter::default();
    let mut model = "mock".to_string();
    if provider_adapter == "openai-compatible" {
        model = std::env::var("PAA_MODEL_ID").expect("set PAA_MODEL_ID");
        let base_url = std::env::var("PAA_MODEL_API_BASE_URL").expect("set PAA_MODEL_API_BASE_URL");
        let limit: usize = std::env::var("PAA_MODEL_CONTEXT_LIMIT")
            .expect("set PAA_MODEL_CONTEXT_LIMIT")
            .parse()
            .expect("valid PAA_MODEL_CONTEXT_LIMIT");
        assert!(
            !model.is_empty() && !base_url.is_empty() && limit > 0,
            "model profile must be complete"
        );
        openai.provider_profiles.insert(
            "local".into(),
            ProviderProfile {
                base_url,
                api_key_env: Some("PAA_MODEL_API_KEY".into()),
                model_context_limits: HashMap::from([(model.clone(), limit)]),
            },
        );
    }
    let adapter = load_model_adapter(&provider_adapter, openai).expect("configured adapter");
    let memory_root = std::env::var("RUNTIME_MEMORY_ROOT").unwrap_or_else(|_| ".paa-memory".into());
    let mut app = Gateway::new(
        AgentLoop::new(adapter, Vec::new()),
        Some(paa_rust_architecture_template::memory::Store::new(
            &memory_root,
        )),
    );
    app.current_model = model;
    app.current_provider = "local".into();
    app.enable_persistence(&memory_root)
        .expect("load conversation memory");
    let gateway = Arc::new(Mutex::new(app));
    let listener = TcpListener::bind(format!("{host}:{port}")).expect("bind server");
    println!("PAA Rust template listening at http://{host}:{port}");
    println!("Try: curl http://{host}:{port}/health");
    for stream in listener.incoming().flatten() {
        let gateway = Arc::clone(&gateway);
        let api_key = Arc::clone(&api_key);
        std::thread::spawn(move || handle_connection(stream, gateway, &api_key));
    }
}
fn error_response(status: u16, code: &str, message: &str) -> Response {
    Response {
        status,
        headers: Map::new(),
        body: json!({"code": code, "message": message}),
        stream: Vec::new(),
    }
}
fn handle_connection(mut stream: TcpStream, gateway: Arc<Mutex<Gateway>>, api_key: &str) {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end;
    loop {
        let count = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(count) => count,
        };
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = index + 4;
            break;
        }
        if bytes.len() > 64 * 1024 {
            return;
        }
    }
    let header_text = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
    let mut lines = header_text.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");
    if method == "GET" && path == "/health" {
        let response =
            gateway
                .lock()
                .expect("gateway lock")
                .handle(method, path, None, &Map::new());
        write_response(&mut stream, response);
        return;
    }
    let mut supplied = String::new();
    for line in header_text.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("X-API-Key") {
                supplied = value.trim().to_string();
            }
            if name.eq_ignore_ascii_case("Authorization") {
                if let Some(token) = value.trim().strip_prefix("Bearer ") {
                    supplied = token.to_string();
                }
            }
        }
    }
    if supplied.is_empty() || supplied != api_key {
        write_response(
            &mut stream,
            error_response(401, "unauthenticated", "Authentication required."),
        );
        return;
    }
    let content_length = header_text
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length:")
                .or_else(|| line.strip_prefix("content-length:"))
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > 1024 * 1024 {
        while bytes.len() < header_end + content_length
            && bytes.len() < header_end + 4 * 1024 * 1024
        {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            }
        }
        write_response(
            &mut stream,
            error_response(413, "request_too_large", "Request body too large."),
        );
        return;
    }
    if method == "POST" && content_length == 0 {
        write_response(
            &mut stream,
            error_response(411, "length_required", "Content-Length required."),
        );
        return;
    }
    while bytes.len() < header_end + content_length {
        let count = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(count) => count,
        };
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > header_end + 1024 * 1024 {
            return;
        }
    }
    let request = String::from_utf8_lossy(&bytes);
    let mut sections = request.splitn(2, "\r\n\r\n");
    let _head = sections.next().unwrap_or_default();
    let body_text = sections.next().unwrap_or_default();
    let mut headers = Map::new();
    headers.insert("X-Actor-ID".into(), Value::String("local-owner".into()));
    headers.insert(
        "X-Actor-Permissions".into(),
        Value::String("memory:read,memory:write".into()),
    );
    let body = if body_text.trim().is_empty() {
        None
    } else {
        serde_json::from_str(body_text).ok()
    };
    let response = gateway
        .lock()
        .expect("gateway lock")
        .handle(method, path, body, &headers);
    write_response(&mut stream, response);
}
fn write_response(stream: &mut TcpStream, response: Response) {
    let is_stream =
        response.headers.get("Content-Type").and_then(Value::as_str) == Some("text/event-stream");
    let mut output = format!(
        "HTTP/1.1 {} {}\r\n",
        response.status,
        if response.status < 400 { "OK" } else { "Error" }
    );
    for (key, value) in response.headers {
        if let Some(value) = value.as_str() {
            output.push_str(&format!("{key}: {value}\r\n"));
        }
    }
    let body = if is_stream {
        response
            .stream
            .iter()
            .map(|event| {
                format!(
                    "event: {}\ndata: {}\n\n",
                    event.event,
                    serde_json::to_string(&event.data).unwrap_or_default()
                )
            })
            .collect::<String>()
    } else {
        serde_json::to_string(&response.body).unwrap_or_else(|_| "{}".into())
    };
    output.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    ));
    let _ = stream.write_all(output.as_bytes());
}
