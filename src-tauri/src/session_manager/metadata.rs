use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WindowState {
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub maximized: bool,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct MetadataStore {
    #[serde(default)]
    pub sessions: HashMap<String, SessionMetadata>,
    #[serde(default)]
    pub pinned_folders: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_state: Option<WindowState>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct SessionMetadata {
    #[serde(default)]
    pub starred: bool,
}

pub struct MetadataManager {
    store: Mutex<MetadataStore>,
    path: PathBuf,
}

impl MetadataManager {
    pub fn new(path: PathBuf) -> Self {
        let store = if path.exists() {
            fs::read_to_string(&path)
                .ok()
                .and_then(|content| serde_json::from_str(&content).ok())
                .unwrap_or_default()
        } else {
            MetadataStore::default()
        };
        Self {
            store: Mutex::new(store),
            path,
        }
    }

    pub fn get_metadata(&self) -> MetadataStore {
        self.store.lock().unwrap().clone()
    }

    pub fn set_session_starred(&self, session_key: &str, starred: bool) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        if starred {
            store
                .sessions
                .entry(session_key.to_string())
                .or_insert_with(SessionMetadata::default)
                .starred = true;
        } else {
            if let Some(meta) = store.sessions.get_mut(session_key) {
                meta.starred = false;
                // Clean up empty entries
                if meta.starred == false {
                    store.sessions.remove(session_key);
                }
            }
        }
        self.save(&store)
    }

    pub fn set_pinned_folders(&self, folders: Vec<String>) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        store.pinned_folders = folders;
        self.save(&store)
    }

    pub fn set_window_state(&self, state: WindowState) -> Result<(), String> {
        let mut store = self.store.lock().unwrap();
        store.window_state = Some(state);
        self.save(&store)
    }

    pub fn get_window_state(&self) -> Option<WindowState> {
        self.store.lock().unwrap().window_state.clone()
    }

    fn save(&self, store: &MetadataStore) -> Result<(), String> {
        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create metadata dir: {e}"))?;
        }
        // Atomic write using tempfile
        let dir = self.path.parent().unwrap_or(&self.path);
        let mut tmp = tempfile::NamedTempFile::new_in(dir)
            .map_err(|e| format!("Failed to create temp file: {e}"))?;
        serde_json::to_writer_pretty(&mut tmp, store)
            .map_err(|e| format!("Failed to serialize metadata: {e}"))?;
        tmp.persist(&self.path)
            .map_err(|e| format!("Failed to persist metadata: {e}"))?;
        Ok(())
    }
}
