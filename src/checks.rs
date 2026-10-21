use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

pub fn run(root: &Path, source_checkout: Option<&Path>) -> Vec<String> {
    let mut errors = check_format(root);
    errors.extend(check_contracts(root, source_checkout));
    errors.extend(check_boundaries(root));
    errors.extend(check_lock_in(root));
    errors
}

pub fn check_format(root: &Path) -> Vec<String> {
    let files = rust_files(&root.join("src"));
    let mut errors = Vec::new();
    if files.is_empty() {
        errors.push("No Rust source files found.".into());
        return errors;
    }
    let mut command = Command::new("rustfmt");
    command.arg("--check").args(&files);
    if let Ok(output) = command.output() {
        if !output.status.success() {
            errors.push(format!(
                "rustfmt check failed: {}",
                String::from_utf8_lossy(&output.stdout).trim()
            ));
        }
    } else {
        errors.push("rustfmt is required for the Rust template checks.".into());
    }
    errors
}

pub fn check_contracts(root: &Path, source_checkout: Option<&Path>) -> Vec<String> {
    let mut errors = Vec::new();
    let lock_path = root.join("paa.lock.json");
    let lock: Value = match fs::read_to_string(&lock_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
    {
        Some(value) => value,
        None => return vec!["paa.lock.json is not valid JSON.".into()],
    };
    if lock["repository"] != "https://github.com/Personal-AI-Architecture/the-architecture.git" {
        errors
            .push("paa.lock.json repository must point to the PAA architecture repository.".into());
    }
    if lock["ref"] != "main" {
        errors.push("paa.lock.json ref must be main.".into());
    }
    let commit = lock["commit"].as_str().unwrap_or_default();
    if commit.len() != 40
        || !commit
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        errors.push("paa.lock.json commit must be a full 40-character SHA.".into());
    }
    if lock["source_root"] != "vendor/paa-contracts" {
        errors.push("paa.lock.json source_root must be vendor/paa-contracts.".into());
    }
    let files = lock["contract_files"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let hashes = lock["sha256"].as_object().cloned().unwrap_or_default();
    for relative_value in files {
        let Some(relative) = relative_value.as_str() else {
            continue;
        };
        let vendor = root.join("vendor/paa-contracts").join(relative);
        let mirror = root.join(relative);
        let vendor_bytes = match fs::read(&vendor) {
            Ok(bytes) => bytes,
            Err(_) => {
                errors.push(format!("Missing vendored contract: {relative}."));
                continue;
            }
        };
        let mut hasher = Sha256::new();
        hasher.update(&vendor_bytes);
        let actual = format!("{:x}", hasher.finalize());
        if hashes.get(relative).and_then(Value::as_str) != Some(actual.as_str()) {
            errors.push(format!("Hash drift in vendored contract: {relative}."));
        }
        if fs::read(&mirror).ok().as_deref() != Some(vendor_bytes.as_slice()) {
            errors.push(format!(
                "Local contract mirror differs from vendored source: {relative}."
            ));
        }
        if let Some(source) = source_checkout {
            let output = Command::new("git")
                .args([
                    "-C",
                    source.to_str().unwrap_or_default(),
                    "show",
                    &format!("{commit}:{relative}"),
                ])
                .output();
            if output
                .as_ref()
                .map(|result| !result.status.success() || result.stdout != vendor_bytes)
                .unwrap_or(true)
            {
                errors.push(format!(
                    "Vendored contract differs from {commit}:{relative}."
                ));
            }
        }
    }
    let gateway = read(root, "specs/openapi/gateway-api.yaml");
    let engine = read(root, "specs/openapi/gateway-engine.yaml");
    let model = read(root, "specs/openapi/model-api.yaml");
    for route in [
        "  /conversations/messages:",
        "  /conversations:",
        "  /conversations/{id}:",
        "  /conversations/{id}/messages:",
    ] {
        if !gateway.contains(route) {
            errors.push(format!("Gateway API path is missing: {route}."));
        }
    }
    if gateway.contains("  /chat:") || gateway.contains("conversation_id}") {
        errors.push("Gateway API contains a non-PAA route or path parameter.".into());
    }
    for marker in [
        "    SendMessageRequest:",
        "    ConversationList:",
        "    Conversation:",
        "    ClientSSEStream:",
    ] {
        if !gateway.contains(marker) {
            errors.push(format!("Gateway schema is missing: {marker}."));
        }
    }
    for marker in [
        "/engine/chat:",
        "EngineRequest:",
        "RequestMetadata:",
        "SSEStream:",
        "X-Actor-ID",
        "X-Actor-Permissions",
    ] {
        if !engine.contains(marker) {
            errors.push(format!("Gateway-Agent Loop contract is missing: {marker}."));
        }
    }
    for marker in ["/completions:", "CompletionRequest:", "CompletionStream:"] {
        if !model.contains(marker) {
            errors.push(format!("Model API contract is missing: {marker}."));
        }
    }
    if model.contains("  /model/chat/completions:") {
        errors.push("Model API promotes a provider-native route into the PAA contract.".into());
    }
    for (relative, required) in [
        (
            "specs/schemas/configuration.json",
            vec![
                "memory_root",
                "provider_adapter",
                "auth_mode",
                "tool_sources",
            ],
        ),
        (
            "specs/schemas/identity.json",
            vec!["id", "type", "display_name", "status", "created"],
        ),
        ("specs/schemas/message.json", vec!["role", "content"]),
        (
            "specs/schemas/policy.json",
            vec!["subject", "resource", "action", "effect"],
        ),
        (
            "specs/schemas/tool-definition.json",
            vec!["name", "description", "parameters"],
        ),
    ] {
        let schema: Value = match serde_json::from_str(&read(root, relative)) {
            Ok(value) => value,
            Err(_) => {
                errors.push(format!("{relative} is not valid JSON."));
                continue;
            }
        };
        let required_values = schema["required"].as_array().cloned().unwrap_or_default();
        if schema["type"] != "object" {
            errors.push(format!("{relative} must define an object schema."));
        }
        for field in required {
            if !required_values
                .iter()
                .any(|value| value.as_str() == Some(field))
            {
                errors.push(format!("{relative} is missing required field: {field}."));
            }
        }
    }
    errors
}

pub fn check_boundaries(root: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let internal = root.join("src");
    for path in rust_files(&internal) {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(&path).unwrap_or_default();
        let core_component = [
            "src/config/",
            "src/engine/",
            "src/gateway/",
            "src/memory/",
            "src/types/",
        ]
        .iter()
        .any(|prefix| relative.starts_with(prefix));
        if core_component {
            for token in [
                "/chat/completions",
                "api_base_url",
                "api_key",
                "openai-compatible",
            ] {
                if source.contains(token) {
                    errors.push(format!("Provider-specific token {token:?} must stay within src/adapters/: {relative}"));
                }
            }
        }
    }
    let metadata = read(root, "src/types/mod.rs");
    if let Some(start) = metadata.find("pub struct EngineRequestMetadata") {
        let section = &metadata[start
            ..metadata[start..]
                .find('}')
                .map(|end| end + start)
                .unwrap_or(metadata.len())];
        for forbidden in ["provider_adapter", "model", "tools"] {
            if section.contains(forbidden) {
                errors.push(format!("EngineRequestMetadata must not include runtime reconfiguration key: {forbidden}"));
            }
        }
    } else {
        errors.push("EngineRequestMetadata contract is missing.".into());
    }
    let gateway = read(root, "src/gateway/mod.rs");
    for marker in ["/conversations/messages", "/conversations", "/messages"] {
        if !gateway.contains(marker) {
            errors.push(format!("Gateway runtime is missing route marker: {marker}"));
        }
    }
    errors
}

pub fn check_lock_in(root: &Path) -> Vec<String> {
    [
        "src/adapters/mod.rs",
        "src/adapters/mock.rs",
        "src/adapters/openai_compatible.rs",
        "src/engine/mod.rs",
        "src/types/mod.rs",
        "src/gateway/mod.rs",
    ]
    .iter()
    .filter(|relative| !root.join(relative).is_file())
    .map(|relative| format!("Missing required file: {relative}"))
    .collect()
}

fn read(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).unwrap_or_default()
}
fn rust_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(rust_files(&path));
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}
