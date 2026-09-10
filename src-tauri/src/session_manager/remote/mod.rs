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
//! - [`scan`] — the phase 3 batch scan: discovery exec + batch-metadata
//!   exec + temp-file bridge into the local provider parsers, provider
//!   probing, and the disconnect fallback. Tauri-free; the command layer
//!   owns heal execution and event emission.
//!
//! ADR 0007 discipline (batch / cache / drop) attribution of this
//! layer's operations:
//! - `batch_metadata` / `exec_script` — **batch** (one exec round-trip
//!   for N files; the P1 real-alias benchmark: ~14 ms/file vs ~710 ms
//!   per-file);
//! - `fetch_to_local` — **cache** (full transfer once, then
//!   mtime+size-gated free re-opens);
//! - `fetch_index_incremental` — **batch** for append-only files
//!   (offset read of the appended suffix only).
//!
//! Configuration comes from the settings-core `SshSource` /
//! `SourceAuth` types — this layer never invents its own config shape.

mod cache;
mod error;
mod frame;
mod scan;
mod session;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

pub use error::RemoteError;
pub use scan::{RemoteSourceResult, SessionBatchFetch};
pub use session::RemoteSession;
// Re-exported for consumers of the scan line (batch blob shape and the
// bridge window constants). Exercised by tests.
#[allow(unused_imports)]
pub use frame::{FileMetadataBlob, HEAD_MAX, TAIL_MAX};

use crate::session_manager::settings::SshSource;
use crate::session_manager::types::{SessionHandle, SessionLocator, SessionMeta};

/// A remote absolute file path (as seen on the SSH host).
pub type RemotePath = String;

/// Bridge a Remote-locator handle to a File-locator handle backed by the
/// transient cache (ADR 0007 "cache" exit — the remote source's ONLY
/// content-read path; phase 4 session-open wiring).
///
/// Semantics:
/// - Non-Remote locators return `Ok(None)` — the caller keeps the original
///   handle and nothing is touched (local sessions pay zero cost).
/// - `source_id` is resolved against the caller-injected enabled ssh
///   entries (the command layer reads `SettingsManager::enabled_sources()`
///   up front so this function stays Tauri-free and Send).
/// - `fetch_to_local(path, None)` performs the mtime+size-gated cache
///   check: a cache hit costs no SFTP round-trip, a miss transfers the
///   file once. `known_attrs = None` because the open path holds no
///   scanned blob attributes.
/// - The returned handle keeps the ORIGINAL `provider_id`/`session_id`:
///   the cache file holds the remote provider's bytes, and the parser
///   choice is bound to the provider id, not to the path.
///
/// Both `get_session_messages` and `get_session_detail` share this bridge;
/// for the detail path the raw-content fallback also flows through the
/// File-locator handle (its default implementation reads `file_path()`,
/// which now resolves to the local cache copy).
pub async fn resolve_remote_to_local(
    ssh_sources: &[SshSource],
    pool: &RemoteSessionPool,
    handle: &SessionHandle,
) -> Result<Option<SessionHandle>, String> {
    let (source_id, remote_path) = match &handle.locator {
        SessionLocator::Remote { source_id, path } => (source_id, path),
        _ => return Ok(None),
    };
    let source = ssh_sources
        .iter()
        .find(|s| &s.id == source_id)
        .ok_or_else(|| format!("Remote source `{source_id}` is not configured or disabled"))?;
    let session = pool
        .get(source)
        .await
        .map_err(|e| format!("Remote source `{source_id}` unreachable: {e}"))?;
    let local = session
        .fetch_to_local(remote_path, None)
        .await
        .map_err(|e| format!("Failed to fetch remote session `{remote_path}`: {e}"))?;
    Ok(Some(SessionHandle {
        provider_id: handle.provider_id.clone(),
        session_id: handle.session_id.clone(),
        locator: SessionLocator::File {
            path: local.to_string_lossy().into_owned(),
        },
    }))
}

/// Per-source session cache: at most one live `RemoteSession` per
/// source id, lazily connected, guarded so concurrent callers of the
/// same source share the connection instead of opening new ones (spec
/// edge case "concurrent fetch of the same source"; ADR 0007 evidence
/// 02 hard requirement 2 — single-connection concurrency).
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
    pub async fn get(
        &self,
        source: &crate::session_manager::settings::SshSource,
    ) -> Result<Arc<RemoteSession>, RemoteError> {
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
    // No production caller yet: the "reconnect" action is phase 4 UI
    // work (the disconnect fallback below covers v1 listing).
    #[allow(dead_code)]
    pub async fn drop_source(&self, source_id: &str) {
        self.sessions.lock().await.remove(source_id);
    }
}

/// Managed state for the remote scan line (phase 3 Tauri wiring): the
/// per-source session pool plus each source's last successful scan.
///
/// The last-scan map is the v1 disconnect fallback (decision: a dead
/// source serves its cached list with a warn instead of failing or
/// emptying the whole session list). `SessionMeta` is deliberately NOT
/// extended with a stale marker — surfacing staleness in the UI is
/// phase 4.
pub struct RemoteScanState {
    /// Per-source live sessions (see [`RemoteSessionPool`]).
    pub pool: RemoteSessionPool,
    /// source id → last successful scan result.
    last_scan: Arc<StdMutex<HashMap<String, Vec<SessionMeta>>>>,
}

impl Default for RemoteScanState {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteScanState {
    pub fn new() -> Self {
        Self::with_pool(RemoteSessionPool::new())
    }

    /// Test seam: explicit pool (cache base).
    pub fn with_pool(pool: RemoteSessionPool) -> Self {
        Self {
            pool,
            last_scan: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    /// Last successful scan for a source (empty when none) — served
    /// when the pool cannot connect at all.
    pub fn cached_sessions(&self, source_id: &str) -> Vec<SessionMeta> {
        self.last_scan
            .lock()
            .expect("remote scan cache lock")
            .get(source_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Scan one source end-to-end: builds a [`SessionBatchFetch`] on the
    /// async side (it captures the ambient tokio handle), runs the scan
    /// core + provider parsers on the blocking pool (temp files and
    /// parser IO are blocking), then applies the disconnect fallback.
    ///
    /// The returned [`RemoteSourceResult`] carries the auto-heal
    /// DECISION — executing `heal_provider_hint` and emitting
    /// `settings-changed` is the command layer's job (this module stays
    /// Tauri-free).
    pub async fn scan_source(
        &self,
        registry: &Arc<crate::session_manager::providers::ProviderRegistry>,
        session: Arc<RemoteSession>,
        source: &crate::session_manager::settings::SshSource,
    ) -> RemoteSourceResult {
        let fetch = SessionBatchFetch::new(session);
        let registry = Arc::clone(registry);
        let last_scan = Arc::clone(&self.last_scan);
        let source = source.clone();
        let source_id = source.id.clone();
        let join = tokio::task::spawn_blocking(move || {
            // StdMutex guard moved into the closure: lock scope == task
            // scope, and the task never awaits while holding it.
            let mut guard = last_scan.lock().expect("remote scan cache lock");
            scan::scan_source_with_fallback(&mut guard, &registry, &fetch, &source)
        })
        .await;
        match join {
            Ok(result) => result,
            Err(err) => {
                // Blocking task panicked/joined-failed: same degradation
                // as a transport failure — cached list, never an error.
                log::warn!("remote scan: blocking task failed ({err}) — serving cached list");
                RemoteSourceResult::fallback(self.cached_sessions(&source_id))
            }
        }
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

    // ── resolve_remote_to_local offline behavior ────────────────────────

    fn ssh_source(id: &str) -> SshSource {
        SshSource {
            id: id.to_string(),
            label: None,
            host: "h".to_string(),
            port: 22,
            user: "u".to_string(),
            root: "~/.claude/projects".to_string(),
            auth: crate::session_manager::settings::SourceAuth::Agent,
            provider_hint: None,
            enabled: true,
            extra: std::collections::BTreeMap::new(),
        }
    }

    fn remote_handle(source_id: &str) -> SessionHandle {
        SessionHandle {
            provider_id: "claude".to_string(),
            session_id: "remote-1".to_string(),
            locator: SessionLocator::Remote {
                source_id: source_id.to_string(),
                path: "/home/u/.claude/projects/p/remote-1.jsonl".to_string(),
            },
        }
    }

    #[tokio::test]
    async fn resolve_bridge_passes_non_remote_handles_through_untouched() {
        let pool = RemoteSessionPool::with_cache_base(PathBuf::from("Z:/nope"));
        let handle = SessionHandle {
            provider_id: "claude".to_string(),
            session_id: "s".to_string(),
            locator: SessionLocator::File {
                path: "/local/s.jsonl".to_string(),
            },
        };
        // Ok(None) BEFORE any connection attempt — the pool would fail on
        // the bogus cache base if it were touched.
        let bridged = resolve_remote_to_local(&[], &pool, &handle)
            .await
            .expect("no error for local handle");
        assert!(bridged.is_none());
    }

    #[tokio::test]
    async fn resolve_bridge_rejects_unknown_source_before_connecting() {
        // No ssh entries configured: the lookup must fail BEFORE the pool
        // tries to connect (offline-safe by construction).
        let pool = RemoteSessionPool::with_cache_base(PathBuf::from("Z:/nope"));
        let err = resolve_remote_to_local(&[], &pool, &remote_handle("ghost"))
            .await
            .expect_err("unknown source must fail");
        assert!(err.contains("not configured"), "unexpected: {err}");
    }

    #[tokio::test]
    async fn resolve_bridge_fails_closed_on_unreachable_source() {
        // Source exists but the host is unreachable: the error names the
        // source instead of leaking a raw transport error.
        let pool = RemoteSessionPool::with_cache_base(PathBuf::from("Z:/nope"));
        let err = resolve_remote_to_local(&[ssh_source("srv")], &pool, &remote_handle("srv"))
            .await
            .expect_err("bogus host must fail");
        assert!(err.contains("unreachable"), "unexpected: {err}");
    }

    #[test]
    fn scan_state_cache_is_empty_until_populated() {
        let state = RemoteScanState::with_pool(RemoteSessionPool::with_cache_base(PathBuf::from(
            "Z:/nope",
        )));
        assert!(state.cached_sessions("none").is_empty());
        state
            .last_scan
            .lock()
            .unwrap()
            .insert("srv".to_string(), Vec::new());
        // Present-but-empty and absent are both served as empty lists;
        // the distinction lives in `from_cache` of RemoteSourceResult.
        assert!(state.cached_sessions("srv").is_empty());
    }
}
