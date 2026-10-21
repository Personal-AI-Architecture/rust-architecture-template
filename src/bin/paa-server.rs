use paa_rust_architecture_template::{
    adapters::{load_model_adapter, OpenAiCompatibleAdapter},
    engine::AgentLoop,
    gateway::{Gateway, Response},
};
use serde_json::{Map, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
};

fn main() {
    let mut args = std::env::args().skip(1);
    let host = args.next().unwrap_or_else(|| "127.0.0.1".into());
    let port = args.next().unwrap_or_else(|| "8000".into());
    let adapter =
        load_model_adapter("mock", OpenAiCompatibleAdapter::default()).expect("mock adapter");
    let gateway = Arc::new(Mutex::new(Gateway::new(
        AgentLoop::new(adapter, Vec::new()),
        None,
    )));
    let listener = TcpListener::bind(format!("{host}:{port}")).expect("bind server");
    println!("PAA Rust template listening at http://{host}:{port}");
    println!("Try: curl http://{host}:{port}/health");
    for stream in listener.incoming().flatten() {
        let gateway = Arc::clone(&gateway);
        std::thread::spawn(move || handle_connection(stream, gateway));
    }
}
fn handle_connection(mut stream: TcpStream, gateway: Arc<Mutex<Gateway>>) {
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
    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let content_length = header_text
        .lines()
        .find_map(|line| {
            line.strip_prefix("Content-Length:")
                .or_else(|| line.strip_prefix("content-length:"))
        })
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() < header_end + content_length {
        let count = match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return,
            Ok(count) => count,
        };
        bytes.extend_from_slice(&buffer[..count]);
    }
    let request = String::from_utf8_lossy(&bytes);
    let mut sections = request.splitn(2, "\r\n\r\n");
    let head = sections.next().unwrap_or_default();
    let body_text = sections.next().unwrap_or_default();
    let mut lines = head.lines();
    let Some(request_line) = lines.next() else {
        return;
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("GET");
    let path = parts.next().unwrap_or("/");
    let mut headers = Map::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().into(), Value::String(value.trim().into()));
        }
    }
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
