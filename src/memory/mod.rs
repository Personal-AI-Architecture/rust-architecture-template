use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::PathBuf;
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Preferences {
    pub model: String,
    pub active_provider_profile: String,
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
    pub fn read_preferences(&self) -> std::io::Result<Option<Preferences>> {
        let path = PathBuf::from(&self.memory_root).join("preferences.json");
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let preference: Preferences =
            serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        if preference.model.is_empty() || preference.active_provider_profile.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid owner preferences",
            ));
        }
        Ok(Some(preference))
    }
    pub fn write_preferences(&self, preference: &Preferences) -> std::io::Result<()> {
        if preference.model.is_empty() || preference.active_provider_profile.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Model and provider are required",
            ));
        }
        std::fs::create_dir_all(&self.memory_root)?;
        let path = PathBuf::from(&self.memory_root).join("preferences.json");
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(
            &temporary,
            serde_json::to_vec(preference).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(temporary, path)
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
        let preferences = self.read_preferences().ok().flatten();
        serde_json::json!({"memory_root": self.memory_root, "entries": entries, "conversations": conversations, "history": history, "preferences": preferences})
    }
}
