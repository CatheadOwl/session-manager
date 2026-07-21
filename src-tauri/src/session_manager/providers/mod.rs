pub mod claude;
pub mod codex;
pub mod gemini;
pub mod hermes;
pub mod openclaw;
pub mod opencode;
pub mod qoder;
pub mod utils;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::types::{SessionMessage, SessionMeta};

/// The core abstraction for a session provider (e.g., Claude, Cursor, Copilot).
///
/// Each provider knows how to discover, parse, load, and move its sessions.
/// All methods accept `&self` — the registry owns the provider and guarantees
/// it lives for the application's lifetime.
pub trait SessionProvider: Send + Sync {
    /// Provider identifier, e.g. "claude".
    fn id(&self) -> &str;

    /// All root directories this provider stores sessions under.
    fn roots(&self) -> Vec<PathBuf>;

    /// Scan a single root directory for all session files.
    fn scan_sessions(&self, root: &Path) -> Vec<SessionMeta>;

    /// Load parsed messages from a session file.
    fn load_messages(&self, path: &Path) -> Result<Vec<SessionMessage>, String>;

    /// Load raw content fallback (last prompt / AI title) when messages are empty.
    fn load_raw_content_fallback(&self, path: &Path) -> Result<Option<String>, String>;

    /// Parse session metadata from a JSONL file path.
    fn parse_session(&self, path: &Path) -> Option<SessionMeta>;

    /// Move a session file (and its sidecar) to a destination directory.
    fn move_session(&self, source: &Path, dest: &Path) -> Result<(), String>;

    /// Validate that a session file actually belongs to the expected session_id.
    fn validate_session_id(&self, source: &Path, expected_id: &str) -> Result<(), String> {
        let meta = self
            .parse_session(source)
            .ok_or_else(|| format!("Failed to parse session metadata: {}", source.display()))?;
        if meta.session_id != expected_id {
            return Err(format!(
                "Session ID mismatch: expected {expected_id}, found {}",
                meta.session_id
            ));
        }
        Ok(())
    }

    /// Extract user input text events from a session file, in chronological order.
    /// Used by the fork tree to compute hash chains for fork detection.
    fn user_events(&self, _path: &Path) -> Result<Vec<String>, String> {
        Err(format!(
            "user_events not supported for provider {}",
            self.id()
        ))
    }

    /// Extract user events with their UUIDs, for fork tree uuid-chain matching.
    /// Returns (text, uuid) pairs in chronological order.
    /// The default implementation wraps user_events(), pairing each text with empty uuid.
    fn user_events_with_uuid(&self, path: &Path) -> Result<Vec<(String, String)>, String> {
        self.user_events(path)
            .map(|events| events.into_iter().map(|t| (t, String::new())).collect())
    }
}

/// A registry of known providers, keyed by their id.
///
/// Built once at app startup and shared via `Arc<ProviderRegistry>` as managed Tauri state.
/// Providers are registered in `lib.rs::setup` and never modified afterward.
///
/// Iteration order (`all()`) follows registration order — the first registered provider
/// is checked first in `parse_session_meta`. This is important because providers with
/// broader filename-based fallback detection should be registered later.
pub struct ProviderRegistry {
    providers: HashMap<String, Box<dyn SessionProvider>>,
    order: Vec<String>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            order: Vec::new(),
        }
    }

    pub fn register(&mut self, provider: Box<dyn SessionProvider>) {
        let id = provider.id().to_string();
        self.order.push(id.clone());
        self.providers.insert(id, provider);
    }

    /// Look up a provider by id. Returns `Err` if not found.
    pub fn get(&self, id: &str) -> Result<&dyn SessionProvider, String> {
        self.providers
            .get(id)
            .map(|p| p.as_ref())
            .ok_or_else(|| format!("Unknown provider: {id}"))
    }

    /// Iterate over all registered providers in registration order.
    #[allow(dead_code)]
    pub fn all(&self) -> impl Iterator<Item = &dyn SessionProvider> {
        self.order
            .iter()
            .filter_map(move |id| self.providers.get(id))
            .map(|p| p.as_ref())
    }

    /// Return the number of registered providers.
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.providers.len()
    }
}
