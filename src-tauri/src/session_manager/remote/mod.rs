//! SSH remote-source connection layer (read-only data source, ADR 0007
//! / ADR 0008 remote v1).
//!
//! Submodules:
//! - [`error`] — `RemoteError`, the reaction-shaped failure taxonomy;
//! - [`frame`] — the binary-safe batch-metadata framing protocol and
//!   script builder (pure, offline-testable);
//! - [`cache`] — the transient local cache for fully-fetched files
//!   (pure path/store logic, offline-testable);
//! - `session` — the russh transport: connect/auth/known_hosts, the
//!   batch exec channel, SFTP fetch. ALL exec/SFTP call sites in the
//!   product live there.
//!
//! ADR 0007 discipline (batch / cache / drop) attribution of this
//! layer's operations:
//! - `batch_metadata` — **batch** (one exec round-trip for N files;
//!   the P1 real-alias benchmark: ~14 ms/file vs ~710 ms per-file);
//! - `fetch_to_local` — **cache** (full transfer once, then
//!   mtime+size-gated free re-opens);
//! - `fetch_index_incremental` — **batch** for append-only files
//!   (offset read of the appended suffix only).
//!
//! Configuration comes from the settings-core `SshSource` /
//! `SourceAuth` types — this layer never invents its own config shape.

// No production caller yet: the consumer is the remote scan/fetch line
// (phase 3 wiring into Tauri managed state). Same standing as
// `heal_provider_hint` — the API contract is fixed and exercised by
// tests; remove this allow when the phase 3 consumer lands.
#![allow(dead_code)]

mod cache;
mod error;
mod frame;
mod session;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub use error::RemoteError;
#[allow(unused_imports)] // re-exported for phase 3 consumers; exercised by tests
pub use frame::{FileMetadataBlob, HEAD_MAX, TAIL_MAX};
pub use session::RemoteSession;

/// A remote absolute file path (as seen on the SSH host).
pub type RemotePath = String;

/// Per-source session cache: at most one live `RemoteSession` per
/// source id, lazily connected, guarded so concurrent callers of the
/// same source share the connection instead of opening new ones (spec
/// edge case "concurrent fetch of the same source"; ADR 0007 evidence
/// 02 hard requirement 2 — single-connection concurrency).
///
/// Wiring into the Tauri managed state is phase 3; the type contract
/// is fixed here.
pub struct RemoteSessionPool {
    sessions: tokio::sync::Mutex<HashMap<String, Arc<RemoteSession>>>,
    cache_base: PathBuf,
}

impl Default for RemoteSessionPool {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteSessionPool {
    pub fn new() -> Self {
        Self::with_cache_base(cache::default_cache_base())
    }

    /// Test seam: explicit cache root.
    pub fn with_cache_base(cache_base: PathBuf) -> Self {
        Self {
            sessions: tokio::sync::Mutex::new(HashMap::new()),
            cache_base,
        }
    }

    /// Get-or-connect the session for one source. A cached session is
    /// reused even after a transport drop — `RemoteSession` performs
    /// its own one-shot reconnect internally; only a permanently dead
    /// session (reconnect failing) surfaces as an error, and the entry
    /// is then dropped so the next call builds a fresh one.
    pub async fn get(&self, source: &crate::session_manager::settings::SshSource) -> Result<Arc<RemoteSession>, RemoteError> {
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.get(&source.id) {
            if !existing.is_closed() {
                return Ok(existing.clone());
            }
            // Dead and left over from a failed retry — rebuild below.
            sessions.remove(&source.id);
        }
        let session = Arc::new(
            RemoteSession::connect_with_cache(source, self.cache_base.clone()).await?,
        );
        sessions.insert(source.id.clone(), session.clone());
        Ok(session)
    }

    /// Drop a source's cached session (next `get` reconnects). For
    /// explicit user-driven "reconnect" and shutdown paths.
    pub async fn drop_source(&self, source_id: &str) {
        self.sessions.lock().await.remove(source_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_starts_empty_and_drops_cleanly() {
        let pool = RemoteSessionPool::with_cache_base(PathBuf::from("Z:/nope"));
        let sessions = pool.sessions.try_lock();
        assert!(sessions.is_ok(), "uncontended lock is acquirable");
        assert!(sessions.unwrap().is_empty());
    }
}
