use crate::types::ToolDefinition;
use reqwest::blocking::Client;
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// Streamable HTTP MCP translation for one configured external source.
pub struct McpToolSource {
    id: String,
    url: String,
    client: Client,
    session_id: Option<String>,
    initialized: bool,
    next_id: u64,
    known: HashSet<String>,
}

impl McpToolSource {
    pub fn new(id: impl Into<String>, url: impl Into<String>) -> Result<Self, String> {
        let id = id.into();
        let url = url.into();
        if id.trim().is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err("Invalid MCP source configuration.".into());
        }
        Ok(Self {
            id,
            url,
            client: Client::new(),
            session_id: None,
            initialized: false,
            next_id: 1,
            known: HashSet::new(),
        })
    }

    fn send(&mut self, method: &str, params: Value, notification: bool) -> Result<Value, String> {
        let payload = if notification {
            json!({"jsonrpc":"2.0","method":method})
        } else {
            let id = self.next_id;
            self.next_id += 1;
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
        };
        let mut request = self
            .client
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-03-26")
            .json(&payload);
        if let Some(session_id) = &self.session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
        let response = request
            .send()
            .map_err(|_| "MCP request failed.".to_owned())?;
        if !response.status().is_success() {
            return Err(format!("MCP request failed ({}).", response.status()));
        }
        if let Some(session_id) = response
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|value| value.to_str().ok())
        {
            self.session_id = Some(session_id.to_owned());
        }
        if notification {
            return Ok(Value::Null);
        }
        let is_sse = response
            .headers()
            .get("Content-Type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/event-stream"));
        let body = response
            .text()
            .map_err(|_| "Invalid MCP response.".to_owned())?;
        let data = if is_sse {
            body.lines()
                .find_map(|line| line.strip_prefix("data:"))
                .ok_or("Invalid MCP event stream.")?
                .trim()
        } else {
            body.as_str()
        };
        let message: Value =
            serde_json::from_str(data).map_err(|_| "Invalid MCP response.".to_owned())?;
        message
            .get("result")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(|| "MCP response was invalid or returned an error.".to_owned())
    }

    fn initialize(&mut self) -> Result<(), String> {
        if self.initialized {
            return Ok(());
        }
        self.send("initialize", json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"paa-template","version":"0.1.0"}}), false)?;
        self.send("notifications/initialized", Value::Null, true)?;
        self.initialized = true;
        Ok(())
    }

    pub fn discover(&mut self) -> Result<Vec<ToolDefinition>, String> {
        self.initialize()?;
        self.known.clear();
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let params = cursor
                .as_ref()
                .map_or(json!({}), |value| json!({"cursor": value}));
            let result = self.send("tools/list", params, false)?;
            for item in result["tools"].as_array().into_iter().flatten() {
                let Some(name) = item["name"].as_str().filter(|name| !name.trim().is_empty())
                else {
                    continue;
                };
                self.known.insert(name.to_owned());
                let parameters = item["inputSchema"].as_object().cloned().unwrap_or_default();
                tools.push(ToolDefinition {
                    name: name.into(),
                    description: item["description"].as_str().unwrap_or_default().into(),
                    parameters,
                    source: json!({"id":self.id,"kind":"mcp"}),
                    category: "discoverable".into(),
                    ..Default::default()
                });
            }
            cursor = result["nextCursor"]
                .as_str()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            if cursor.is_none() {
                return Ok(tools);
            }
        }
    }

    pub fn call(&mut self, name: &str, arguments: Map<String, Value>) -> Result<Value, String> {
        if !self.known.contains(name) {
            return Err("Tool was not discovered from this source.".into());
        }
        self.send(
            "tools/call",
            json!({"name":name,"arguments":arguments}),
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::McpToolSource;
    use serde_json::{json, Map};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn binds_configured_identity_and_calls_discovered_tool() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let mut methods = Vec::new();
            for _ in 0..4 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut bytes = vec![0_u8; 8192];
                let count = stream.read(&mut bytes).unwrap();
                let request = String::from_utf8_lossy(&bytes[..count]);
                let method = if request.contains("notifications/initialized") {
                    "notifications/initialized"
                } else if request.contains("tools/list") {
                    "tools/list"
                } else if request.contains("tools/call") {
                    "tools/call"
                } else {
                    "initialize"
                };
                methods.push(method.to_owned());
                let result = match method {
                    "tools/list" => {
                        json!({"tools":[{"name":"search","inputSchema":{"type":"object"},"source":{"id":"spoofed"}}]})
                    }
                    "tools/call" => json!({"content":[{"type":"text","text":"found"}]}),
                    _ => json!({}),
                };
                let body = if method == "notifications/initialized" {
                    String::new()
                } else {
                    json!({"jsonrpc":"2.0","id":1,"result":result}).to_string()
                };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nMcp-Session-Id: session-1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                stream.write_all(response.as_bytes()).unwrap();
            }
            methods
        });
        let mut adapter = McpToolSource::new("external", url).unwrap();
        assert!(adapter.call("search", Map::new()).is_err());
        let definitions = adapter.discover().unwrap();
        assert_eq!(definitions[0].source, json!({"id":"external","kind":"mcp"}));
        assert_eq!(
            adapter.call("search", Map::new()).unwrap()["content"][0]["text"],
            "found"
        );
        assert_eq!(
            server.join().unwrap(),
            [
                "initialize",
                "notifications/initialized",
                "tools/list",
                "tools/call"
            ]
        );
    }
}
