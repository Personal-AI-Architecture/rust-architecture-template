use serde::Serialize;
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default, Serialize)]
pub struct Entry {
    pub kind: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub conversation_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

#[derive(Clone, Debug)]
pub struct Store {
    pub memory_root: String,
    entries: Arc<Mutex<Vec<Entry>>>,
}

impl Store {
    pub fn new(memory_root: impl Into<String>) -> Self {
        Self {
            memory_root: memory_root.into(),
            entries: Arc::new(Mutex::new(Vec::new())),
        }
    }
    pub fn record(&self, entry: Entry) {
        self.entries.lock().expect("memory lock").push(entry);
    }
    pub fn export(&self) -> Value {
        let entries = self.entries.lock().expect("memory lock").clone();
        let conversations: Vec<_> = entries
            .iter()
            .filter(|entry| entry.kind == "conversation")
            .cloned()
            .collect();
        let history: Vec<_> = entries
            .iter()
            .filter(|entry| entry.kind == "message")
            .cloned()
            .collect();
        serde_json::json!({"memory_root": self.memory_root, "entries": entries, "conversations": conversations, "history": history})
    }
}
