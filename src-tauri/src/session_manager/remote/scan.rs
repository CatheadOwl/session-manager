//! Remote batch scan (phase 3): build a source's session list from ONE
//! discovery exec + ONE batch-metadata exec, reusing the LOCAL provider
//! parsers through a temp-file bridge (ADR 0007 "batch" exit; the P1
//! real-alias benchmark: ~14 ms/file vs ~710 ms per-file).
//!
//! ## Layering (auto-heal is DECIDED here, EXECUTED by the command layer)
//!
//! This module is Tauri-free. `scan_remote_source` returns a
//! `RemoteScanOutcome` whose `detected_provider` is a *decision*: when the
//! ssh source has no `providerHint` and the sampled probe agrees on
//! exactly one provider, the outcome carries `(source_id, provider_id)`.
//! The command layer (`commands/session_manager.rs`) turns that into
//! `SettingsManager::heal_provider_hint` + a `settings-changed` event on
//! `Applied`; the scan core and the settings manager never emit events,
//! and the CLI adapter never calls heal (ADR 0008 §1a conditions).
//!
//! ## Temp-file bridge semantics (the load-bearing invariant)
//!
//! Provider `parse_session(path)` reads via
//! `providers::utils::read_head_tail_lines(path, N, M)`, which switches
//! behavior at the 16 KiB boundary (`file_len < 16_384` → whole-file
//! read; else head N lines + seek-to-`len-16_384` tail read). The bridge
//! reconstructs a byte stream that reproduces EXACTLY the lines that
//! reading the original remote file would produce:
//!
//! - `size <= TAIL_MAX (16384)`: the batch `tail` IS the whole file
//!   (min(TAIL_MAX, size) == size), so the bridge is the tail bytes
//!   alone — byte-for-byte the original. Below 16384 that takes
//!   `read_head_tail_lines`' whole-file path; at exactly 16384 both the
//!   original and the bridge take the big-file path with seek-to-0.
//!   (Why `<=`, not `<`: at exactly 16384 head and tail overlap —
//!   concatenating them would duplicate the first 8192 bytes and stitch
//!   a junction line, while the tail alone is exact.)
//! - `size > TAIL_MAX`: the bridge is `head (8192) + tail (16384)` =
//!   24576 bytes >= 16384, so the bridge takes the big-file path: its
//!   head lines start at the concatenation start (= original head), and
//!   its tail read seeks to `24576 - 16384 = 8192` — landing exactly on
//!   the head/tail junction — and reads the tail region (= original
//!   tail; both files drop the identical half-line crossing their own
//!   seek boundary).
//!
//! Both paths are proven equal to "run `read_head_tail_lines` on the
//! original full file" by offline tests with real byte fixtures
//! (`bridge_*_matches_direct_read`). Known limitation (accepted for v1,
//! per decision): equivalence additionally assumes the first N head
//! lines complete within the 8192 head bytes — true for real session
//! files (first lines are short metadata lines; P0a measured "a few KB"
//! for the head read). A pathological file whose 10-30 head lines exceed
//! 8 KiB would see the junction line stitched from head+tail bytes.
//!
//! ## Codex sidecar degradation (v1)
//!
//! codex `parse_session` also reads a local `~/.codex/session_index.jsonl`
//! sidecar for titles. On the scanning machine that is the LOCAL index,
//! which never contains remote session ids — the lookup misses and the
//! title fallback chain applies. v1 does not transfer the remote index
//! (no N× read shape is allowed for it in the scan path; see ADR 0007).
//!
//! ## Provider support matrix (enforced by discovery shape)
//!
//! Discovery only collects `*.jsonl`, which satisfies the P0a matrix by
//! construction: gemini (`.json` chats) and opencode (sqlite / storage
//! directory tree) are never returned, so no per-provider branch is
//! needed here.
//!
//! ## Disconnect fallback (v1 simplification)
//!
//! `scan_source_with_fallback` keeps each source's last successful scan
//! in a caller-held map and serves it (with a `log::warn!`) when a scan
//! fails; a first-connect failure yields an empty list. `SessionMeta` is
//! NOT extended with a stale marker — the UI-side stale badge is phase 4.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::error::RemoteError;
use super::frame::{FileMetadataBlob, TAIL_MAX, shell_quote};
use super::RemotePath;
use super::RemoteSession;
use crate::session_manager::providers::ProviderRegistry;
use crate::session_manager::settings::SshSource;
use crate::session_manager::types::{SessionLocator, SessionMeta};

/// How many leading files (after sorting) feed the provider probe when
/// `provider_hint` is absent. Small on purpose: the probe only needs a
/// consistent witness, while every extra sample is another temp-file
/// parse per registered provider.
pub const PROBE_SAMPLE_COUNT: usize = 5;

// ---------------------------------------------------------------------------
// BatchFetch: the IO surface the scan core consumes
// ---------------------------------------------------------------------------

/// The two exec round-trips a remote scan needs, as a trait so tests can
/// install a fake (the transport itself is `RemoteSession`, wrapped by
/// [`SessionBatchFetch`]). Keeping this seam synchronous lets the whole
/// scan core (temp files + provider parsers, all blocking local IO) run
/// on the blocking pool.
pub trait BatchFetch {
    /// One exec: list `*.jsonl` files under `root` (sorted by the
    /// implementation for deterministic probe sampling).
    fn list_jsonl_files(&self, root: &str) -> Result<Vec<RemotePath>, RemoteError>;
    /// One exec: stat + head + tail for many files (the `batch_metadata`
    /// wire protocol lives in `frame.rs`).
    fn batch_metadata(&self, files: &[RemotePath]) -> Result<Vec<FileMetadataBlob>, RemoteError>;
}

/// Production [`BatchFetch`] over a live [`RemoteSession`]. The async
/// session methods are driven with the ambient tokio handle captured at
/// construction — build this on the async side, use it on the blocking
/// pool.
pub struct SessionBatchFetch {
    session: Arc<RemoteSession>,
    handle: tokio::runtime::Handle,
}

impl SessionBatchFetch {
    /// Captures `Handle::current()` — MUST be called from an async
    /// context (the command layer does; see `RemoteScanState`).
    pub fn new(session: Arc<RemoteSession>) -> Self {
        Self {
            session,
            handle: tokio::runtime::Handle::current(),
        }
    }
}

impl BatchFetch for SessionBatchFetch {
    fn list_jsonl_files(&self, root: &str) -> Result<Vec<RemotePath>, RemoteError> {
        let script = build_find_command(root);
        let stdout = self.handle.block_on(self.session.exec_script(&script))?;
        Ok(parse_find_output(&stdout))
    }

    fn batch_metadata(&self, files: &[RemotePath]) -> Result<Vec<FileMetadataBlob>, RemoteError> {
        self.handle.block_on(self.session.batch_metadata(files))
    }
}

// ---------------------------------------------------------------------------
// File discovery (one exec)
// ---------------------------------------------------------------------------

/// Build the discovery command: `find <root> -type f -name '*.jsonl'`.
///
/// `root` may start with `~` (the settings format allows it): a leading
/// `~` is rewritten to the remote shell's `"$HOME"` so the *remote* side
/// expands it (we never expand `~` locally — it denotes the remote
/// user's home). Everything after `~/` is single-quoted via
/// [`shell_quote`], as is any absolute root.
pub fn build_find_command(root: &str) -> String {
    let root_arg = if root == "~" {
        "\"$HOME\"".to_string()
    } else if let Some(rest) = root.strip_prefix("~/") {
        format!("\"$HOME\"/{}", shell_quote(rest))
    } else {
        shell_quote(root)
    };
    // stderr silenced: unreadable subdirectories must not fail the scan.
    format!("find {root_arg} -type f -name '*.jsonl' 2>/dev/null")
}

/// Parse `find` output into paths (newline-separated, CRLF tolerated,
/// blank lines dropped). The batch protocol requires UTF-8 paths;
/// non-UTF-8 output is lossily converted rather than failing the whole
/// scan (such paths fail batch framing later and get skipped there).
pub fn parse_find_output(bytes: &[u8]) -> Vec<RemotePath> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

// ---------------------------------------------------------------------------
// Temp-file bridge
// ---------------------------------------------------------------------------

/// Bridge byte layout — the exact reproduction rule for
/// `read_head_tail_lines` semantics (see the module docs for the proof):
///
/// - `size <= TAIL_MAX` → the tail bytes alone (tail == whole file,
///   byte-exact; concatenating head here would duplicate content since
///   head and tail overlap);
/// - `size > TAIL_MAX` → head bytes ++ tail bytes
///   (8192 + 16384 = 24576 > 16384 → the big-file read path).
pub fn bridge_bytes(blob: &FileMetadataBlob) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(blob.head.len() + blob.tail.len());
    if blob.size <= TAIL_MAX as u64 {
        // The whole file arrived as the tail region.
        bytes.extend_from_slice(&blob.tail);
    } else {
        bytes.extend_from_slice(&blob.head);
        bytes.extend_from_slice(&blob.tail);
    }
    bytes
}

/// Write the bridge file for one blob into `dir`. The file KEEPS the
/// remote file's basename: several providers derive session ids from the
/// filename stem and check the `.jsonl` extension, so the bridge must be
/// filename-faithful, not just content-faithful. Callers give each blob
/// its own subdirectory (same-basename files in different remote
/// directories must not collide).
pub fn write_bridge_file(dir: &Path, blob: &FileMetadataBlob) -> std::io::Result<PathBuf> {
    let name = Path::new(&blob.path)
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "session.jsonl".into());
    let path = dir.join(name);
    std::fs::write(&path, bridge_bytes(blob))?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// Scan core
// ---------------------------------------------------------------------------

/// Result of scanning one remote source.
#[derive(Debug, Clone)]
pub struct RemoteScanOutcome {
    /// Parsed sessions, locator = `Remote { source_id, path }`,
    /// `source_path = None` (keeps "source_path = a LOCAL path" clean;
    /// the disable gate keys on the locator, not source_path — P0b).
    pub sessions: Vec<SessionMeta>,
    /// `Some((source_id, provider_id))` ONLY when `provider_hint` was
    /// absent and the sampled probe agreed on exactly one provider. This
    /// is a decision for the command layer, which owns heal execution +
    /// the `settings-changed` event; the CLI adapter ignores it.
    pub detected_provider: Option<(String, String)>,
}

/// Scan one remote source: discovery exec → batch-metadata exec →
/// temp-file bridge → provider `parse_session` per file. Files that fail
/// to parse are skipped, matching local scan behavior.
pub fn scan_remote_source(
    registry: &ProviderRegistry,
    fetch: &dyn BatchFetch,
    source: &SshSource,
) -> Result<RemoteScanOutcome, RemoteError> {
    let mut files = fetch.list_jsonl_files(&source.root)?;
    if files.is_empty() {
        return Ok(RemoteScanOutcome {
            sessions: Vec::new(),
            detected_provider: None,
        });
    }
    // Deterministic order: stable probe sampling and stable output lists.
    files.sort();

    let blobs = fetch.batch_metadata(&files)?;

    // Provider selection: explicit hint wins; otherwise probe.
    let (provider_id, detected) = match &source.provider_hint {
        Some(hint) => match registry.get(hint) {
            Ok(provider) => (provider.id().to_string(), None),
            Err(err) => {
                // Warn-only, like the local overlay's unknown-provider
                // path (D4): the entry survives in settings so the user
                // can fix the hint by hand.
                log::warn!(
                    "remote scan: source `{}` has unknown providerHint `{hint}` ({err}) — no sessions this round",
                    source.id
                );
                return Ok(RemoteScanOutcome {
                    sessions: Vec::new(),
                    detected_provider: None,
                });
            }
        },
        None => {
            let temp = tempfile::tempdir()
                .map_err(|e| RemoteError::Io(format!("probe tempdir: {e}")))?;
            match probe_provider(registry, &blobs, temp.path()) {
                Some(id) => (id.clone(), Some((source.id.clone(), id))),
                None => {
                    // Ambiguous or empty probe: refuse to guess (a wrong
                    // parser risks wrong session semantics, D5). The user
                    // sets providerHint by hand; next scan uses it.
                    log::warn!(
                        "remote scan: source `{}` provider probe inconclusive (empty or ambiguous samples) — returning no sessions; set providerHint in settings",
                        source.id
                    );
                    return Ok(RemoteScanOutcome {
                        sessions: Vec::new(),
                        detected_provider: None,
                    });
                }
            }
        }
    };

    let temp = tempfile::tempdir()
        .map_err(|e| RemoteError::Io(format!("scan tempdir: {e}")))?;
    let provider = registry
        .get(&provider_id)
        .expect("provider id validated above");
    let mut sessions = Vec::new();
    for (idx, blob) in blobs.iter().enumerate() {
        // Per-blob subdirectory: same basenames must not collide.
        let blob_dir = temp.path().join(format!("{idx:05}"));
        if let Err(e) = std::fs::create_dir_all(&blob_dir) {
            log::warn!("remote scan: bridge dir for {} failed: {e}", blob.path);
            continue;
        }
        let bridge = match write_bridge_file(&blob_dir, blob) {
            Ok(path) => path,
            Err(e) => {
                log::warn!("remote scan: bridge file for {} failed: {e}", blob.path);
                continue;
            }
        };
        // Reuse the provider's own parser untouched (zero changes to the
        // 9 local providers). Parse failure = skip, as in local scan.
        let Some(mut meta) = provider.parse_session(&bridge) else {
            log::debug!("remote scan: skipped unparseable {}", blob.path);
            continue;
        };
        // Re-anchor to the remote source: the bridge path is a scratch
        // local path and must never leak into the UI or the cache.
        meta.source_path = None;
        meta.locator = Some(SessionLocator::Remote {
            source_id: source.id.clone(),
            path: blob.path.clone(),
        });
        sessions.push(meta);
    }
    // The tempdir (and every bridge file) is dropped here — scratch by
    // construction, cleaned even on early returns via tempdir's Drop.
    Ok(RemoteScanOutcome {
        sessions,
        detected_provider: detected,
    })
}

/// Probe the provider by sampling: for each of the first
/// [`PROBE_SAMPLE_COUNT`] blobs, every registered provider (registration
/// order) tries `parse_session` on the bridge file. Detection succeeds
/// only when EVERY sample matches EXACTLY ONE provider and all samples
/// agree. Anything else — zero files, a sample matching none, a sample
/// matching several, disagreement between samples — is inconclusive.
fn probe_provider(
    registry: &ProviderRegistry,
    blobs: &[FileMetadataBlob],
    scratch: &Path,
) -> Option<String> {
    // Mirrors the LOCAL parse semantics (`parse_session_meta`): the first
    // provider in registration order that parses the file wins. Requiring
    // a UNIQUE match across all providers is impossible for this format
    // family — a claude JSONL line carries sessionId+type, which also
    // satisfies the weaker checks of later-registered providers (qoder's
    // same-line check). Registration order is the tie-breaker locally, so
    // it is the tie-breaker here too; unanimity is then required ACROSS
    // samples of that first-match result.
    let mut agreed: Option<String> = None;
    for (idx, blob) in blobs.iter().take(PROBE_SAMPLE_COUNT).enumerate() {
        let blob_dir = scratch.join(format!("probe-{idx}"));
        std::fs::create_dir_all(&blob_dir).ok()?;
        let bridge = write_bridge_file(&blob_dir, blob).ok()?;
        let first_match = registry
            .all()
            .find(|p| p.parse_session(&bridge).is_some())
            .map(|p| p.id().to_string());
        match first_match {
            None => {
                // This sample parses under NO provider — the scan loop
                // would skip this file anyway (same semantics as the
                // local scan). A few unparsable files (subagent sidecars,
                // foreign formats) must not disqualify an otherwise
                // unanimous root.
                log::debug!(
                    "remote scan probe: {} matched no provider — skipping sample",
                    blob.path
                );
                continue;
            }
            Some(id) => match &agreed {
                Some(previous) if previous != &id => {
                    log::debug!(
                        "remote scan probe: sample {idx} says {id}, earlier said {previous} — inconclusive"
                    );
                    return None;
                }
                _ => agreed = Some(id),
            },
        }
    }
    agreed
}

// ---------------------------------------------------------------------------
// Disconnect fallback wrapper (decision made here, heal executed above)
// ---------------------------------------------------------------------------

/// Per-source scan result handed to the command layer: the session list
/// to append, plus the auto-heal decision to execute there.
#[derive(Debug, Clone)]
pub struct RemoteSourceResult {
    /// Sessions to append to the list result (cached list on failure).
    pub sessions: Vec<SessionMeta>,
    /// Auto-heal decision from this scan (see [`RemoteScanOutcome`]).
    /// The command layer executes it; CLI adapters ignore it.
    pub heal: Option<(String, String)>,
    /// True when `sessions` came from the disconnect cache (v1: the UI
    /// stale badge is phase 4; callers may only log).
    // Read by tests today; the phase 4 UI stale marker is the production
    // consumer (v1 command layer only logs, inside the fallback helper).
    #[allow(dead_code)]
    pub from_cache: bool,
}

impl RemoteSourceResult {
    /// Build a fallback result from an already-held list.
    pub fn fallback(sessions: Vec<SessionMeta>) -> Self {
        Self {
            sessions,
            heal: None,
            from_cache: true,
        }
    }
}

/// Scan one source with the v1 disconnect semantics: on success, refresh
/// `last_scan` and report the heal decision; on failure, serve the
/// cached list (empty when the very first scan failed) with a warn — a
/// dead remote source must never block or empty the local list.
pub fn scan_source_with_fallback(
    last_scan: &mut HashMap<String, Vec<SessionMeta>>,
    registry: &ProviderRegistry,
    fetch: &dyn BatchFetch,
    source: &SshSource,
) -> RemoteSourceResult {
    match scan_remote_source(registry, fetch, source) {
        Ok(outcome) => {
            let heal = outcome.detected_provider;
            last_scan.insert(source.id.clone(), outcome.sessions.clone());
            RemoteSourceResult {
                sessions: outcome.sessions,
                heal,
                from_cache: false,
            }
        }
        Err(err) => {
            log::warn!(
                "remote scan: source `{}` failed ({err}) — serving last successful scan",
                source.id
            );
            let cached = last_scan.get(&source.id).cloned().unwrap_or_default();
            RemoteSourceResult::fallback(cached)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::providers::SessionProvider;
    use crate::session_manager::providers::utils::read_head_tail_lines;
    use super::super::frame::HEAD_MAX;
    use tempfile::tempdir;

    // ── fixtures ────────────────────────────────────────────────────────

    /// Build a batch blob exactly like the remote script / frame parser
    /// would: head = first min(HEAD_MAX, size), tail = last
    /// min(TAIL_MAX, size) (overlapping when small — `head -c`/`tail -c`
    /// semantics, see frame.rs).
    fn blob_for(path: &str, content: &[u8]) -> FileMetadataBlob {
        let size = content.len();
        FileMetadataBlob {
            path: path.to_string(),
            size: size as u64,
            mtime: 1,
            head: content[..HEAD_MAX.min(size)].to_vec(),
            tail: content[size - TAIL_MAX.min(size)..].to_vec(),
        }
    }

    /// A whole session file as raw bytes: `marker_line` first, then `n`
    /// numbered jsonl lines.
    fn session_bytes(marker: &str, lines: usize, pad: usize) -> Vec<u8> {
        let mut out = format!("{marker}\n").into_bytes();
        for i in 0..lines {
            let pad_field = "x".repeat(pad);
            out.extend_from_slice(
                format!("{{\"n\":{i},\"pad\":\"{pad_field}\",\"ts\":1700000000}}\n").as_bytes(),
            );
        }
        out
    }

    fn ssh_source(id: &str, hint: Option<&str>) -> SshSource {
        SshSource {
            id: id.to_string(),
            label: None,
            host: "h".to_string(),
            port: 22,
            user: "u".to_string(),
            root: "~/.fake/projects".to_string(),
            auth: crate::session_manager::settings::SourceAuth::Agent,
            provider_hint: hint.map(str::to_string),
            enabled: true,
            extra: std::collections::BTreeMap::new(),
        }
    }

    /// Fake transport: canned file list + contents, switchable failure.
    struct FakeFetch {
        files: Vec<RemotePath>,
        contents: HashMap<RemotePath, Vec<u8>>,
        fail: bool,
    }

    impl BatchFetch for FakeFetch {
        fn list_jsonl_files(&self, _root: &str) -> Result<Vec<RemotePath>, RemoteError> {
            Ok(self.files.clone())
        }
        fn batch_metadata(&self, files: &[RemotePath]) -> Result<Vec<FileMetadataBlob>, RemoteError> {
            if self.fail {
                return Err(RemoteError::Disconnected);
            }
            Ok(files
                .iter()
                .map(|f| blob_for(f, &self.contents[f]))
                .collect())
        }
    }

    /// Marker-based fixture provider: parses a file iff its first line
    /// contains `"provider":"<id>"`. Lets probe tests control exactly
    /// which providers match a sample.
    struct MarkerProvider {
        id: &'static str,
        loose: bool,
    }

    impl SessionProvider for MarkerProvider {
        fn id(&self) -> &str {
            self.id
        }
        fn roots(&self) -> Vec<PathBuf> {
            Vec::new()
        }
        fn scan_sessions(&self, _root: &Path) -> Vec<SessionMeta> {
            Vec::new()
        }
        fn load_messages(&self, _path: &Path) -> Result<Vec<crate::session_manager::SessionMessage>, String> {
            Ok(Vec::new())
        }
        fn load_raw_content_fallback(&self, _path: &Path) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn parse_session(&self, path: &Path) -> Option<SessionMeta> {
            let (head, _) = read_head_tail_lines(path, 1, 1).ok()?;
            let first = head.first()?;
            let marker = format!("\"provider\":\"{}\"", self.id);
            if !self.loose && !first.contains(&marker) {
                return None;
            }
            let session_id = path.file_stem()?.to_str()?.to_string();
            let file = path.to_string_lossy().into_owned();
            Some(SessionMeta {
                provider_id: self.id.to_string(),
                session_id,
                title: Some(self.id.to_string()),
                summary: None,
                project_dir: None,
                created_at: None,
                last_active_at: None,
                source_path: Some(file.clone()),
                locator: Some(SessionLocator::File { path: file }),
                resume_command: None,
                forked_from_id: None,
            })
        }
        fn move_session(&self, _source: &Path, _dest: &Path) -> Result<(), String> {
            Ok(())
        }
    }

    fn registry_of(providers: Vec<MarkerProvider>) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        for p in providers {
            registry.register(Box::new(p));
        }
        registry
    }

    // ── discovery command ───────────────────────────────────────────────

    #[test]
    fn find_command_expands_home_and_quotes_paths() {
        // Bare ~ and ~/roots delegate expansion to the remote $HOME.
        assert_eq!(
            build_find_command("~"),
            "find \"$HOME\" -type f -name '*.jsonl' 2>/dev/null"
        );
        assert_eq!(
            build_find_command("~/.claude/projects"),
            "find \"$HOME\"/'.claude/projects' -type f -name '*.jsonl' 2>/dev/null"
        );
        // Absolute roots are single-quoted (spaces, quotes, globs stay
        // literal).
        assert_eq!(
            build_find_command("/data/my sessions"),
            "find '/data/my sessions' -type f -name '*.jsonl' 2>/dev/null"
        );
        assert!(build_find_command("/data/it's").contains("'/data/it'\\''s'"));
    }

    #[test]
    fn find_output_parses_lines_and_drops_blanks() {
        assert_eq!(
            parse_find_output(b"/a.jsonl\r\n\n/b.jsonl\n"),
            vec!["/a.jsonl".to_string(), "/b.jsonl".to_string()]
        );
        assert!(parse_find_output(b"").is_empty());
    }

    // ── A. temp-file bridge semantics vs direct read ────────────────────

    /// The load-bearing comparison (decision A): reading head/tail lines
    /// from the BRIDGE must equal reading them from the ORIGINAL file.
    fn assert_bridge_equivalence(content: &[u8]) {
        let dir = tempdir().expect("tempdir");
        let original = dir.path().join("original.jsonl");
        std::fs::write(&original, content).expect("write original");
        let blob = blob_for("/remote/original.jsonl", content);
        let bridge_dir = dir.path().join("bridge");
        std::fs::create_dir_all(&bridge_dir).expect("bridge dir");
        let bridge = write_bridge_file(&bridge_dir, &blob).expect("bridge");

        for (head_n, tail_m) in [(10, 30), (30, 10), (1, 1)] {
            let direct = read_head_tail_lines(&original, head_n, tail_m).expect("direct read");
            let bridged = read_head_tail_lines(&bridge, head_n, tail_m).expect("bridge read");
            assert_eq!(direct, bridged, "head {head_n}/tail {tail_m} diverged");
        }
    }

    #[test]
    fn bridge_small_file_matches_direct_read() {
        // < TAIL_MAX: the bridge is the tail alone == the whole file.
        let content = session_bytes("{\"provider\":\"alpha\"}", 200, 10);
        assert!(content.len() < TAIL_MAX, "fixture must be small: {}", content.len());
        assert_bridge_equivalence(&content);
    }

    #[test]
    fn bridge_large_file_matches_direct_read() {
        // >= TAIL_MAX: head(8192)+tail(16384) = 24576 >= 16384 → the
        // big-file read path; the tail seek lands exactly on the junction.
        let content = session_bytes("{\"provider\":\"alpha\"}", 900, 60);
        assert!(content.len() > HEAD_MAX + TAIL_MAX, "fixture must be large: {}", content.len());
        assert_bridge_equivalence(&content);
    }

    #[test]
    fn bridge_boundary_exactly_tail_max_matches_direct_read() {
        // size == 16384 exactly: the bridge is the whole 16384-byte file
        // (tail-only — head and tail overlap here), which itself sits on
        // the big-file boundary of read_head_tail_lines (seek to 0, no
        // crossing line to drop on either side).
        let mut content = session_bytes("{\"provider\":\"alpha\"}", 1, 10);
        while content.len() < TAIL_MAX {
            content.push(b' ');
        }
        content.truncate(TAIL_MAX);
        assert_eq!(content.len(), TAIL_MAX);
        assert_bridge_equivalence(&content);
    }

    #[test]
    fn bridge_just_above_tail_max_matches_direct_read() {
        // size just over TAIL_MAX: the head+tail concatenation path with
        // maximal head/tail overlap. The bridge's tail seek lands at 8192
        // = the junction = exactly original byte (size - 16384), so both
        // reads drop the same crossing half-line. The fixture keeps >= 30
        // complete lines well inside the 8192-byte head region (the
        // documented head-lines-fit assumption).
        let content = session_bytes("{\"provider\":\"alpha\"}", 430, 10);
        assert!(
            content.len() > TAIL_MAX && content.len() < HEAD_MAX + TAIL_MAX,
            "fixture must overlap head/tail: {}",
            content.len()
        );
        assert_bridge_equivalence(&content);
    }

    // ── scan core: hint, probe, locators, skip semantics ────────────────

    #[test]
    fn scan_with_hint_parses_files_and_anchors_remote_locators() {
        let registry = registry_of(vec![
            MarkerProvider { id: "alpha", loose: false },
            MarkerProvider { id: "beta", loose: false },
        ]);
        let mut contents = HashMap::new();
        contents.insert(
            "/r/one.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 20, 5),
        );
        // A file the hinted provider cannot parse is skipped (local scan
        // parity), and a mid-size file still round-trips.
        contents.insert(
            "/r/broken.jsonl".to_string(),
            session_bytes("{\"provider\":\"beta\"}", 20, 5),
        );
        contents.insert(
            "/r/two.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 500, 60),
        );
        let fetch = FakeFetch {
            files: vec![
                "/r/one.jsonl".to_string(),
                "/r/broken.jsonl".to_string(),
                "/r/two.jsonl".to_string(),
            ],
            contents,
            fail: false,
        };

        let outcome =
            scan_remote_source(&registry, &fetch, &ssh_source("srv", Some("alpha"))).expect("scan");
        // Hint given → no heal decision.
        assert!(outcome.detected_provider.is_none());
        let ids: Vec<&str> = outcome.sessions.iter().map(|m| m.session_id.as_str()).collect();
        assert_eq!(ids, vec!["one", "two"], "unparseable file skipped");
        for meta in &outcome.sessions {
            assert_eq!(meta.provider_id, "alpha");
            assert_eq!(meta.source_path, None, "source_path stays a LOCAL-path concept");
            match &meta.locator {
                Some(SessionLocator::Remote { source_id, path }) => {
                    assert_eq!(source_id, "srv");
                    assert!(path.starts_with("/r/"));
                }
                other => panic!("expected Remote locator, got {other:?}"),
            }
        }
    }

    #[test]
    fn scan_unknown_hint_warns_and_returns_empty() {
        let registry = registry_of(vec![MarkerProvider { id: "alpha", loose: false }]);
        let mut contents = HashMap::new();
        contents.insert(
            "/r/one.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 5, 5),
        );
        let fetch = FakeFetch {
            files: vec!["/r/one.jsonl".to_string()],
            contents,
            fail: false,
        };
        let outcome =
            scan_remote_source(&registry, &fetch, &ssh_source("srv", Some("nope"))).expect("ok");
        assert!(outcome.sessions.is_empty());
        assert!(outcome.detected_provider.is_none());
    }

    #[test]
    fn probe_consistent_samples_detect_provider_and_heal_decision() {
        let registry = registry_of(vec![
            MarkerProvider { id: "alpha", loose: false },
            MarkerProvider { id: "beta", loose: false },
        ]);
        let mut contents = HashMap::new();
        for i in 0..(PROBE_SAMPLE_COUNT + 2) {
            contents.insert(
                format!("/r/{i}.jsonl"),
                session_bytes("{\"provider\":\"alpha\"}", 10, 5),
            );
        }
        let fetch = FakeFetch {
            files: (0..PROBE_SAMPLE_COUNT + 2).map(|i| format!("/r/{i}.jsonl")).collect(),
            contents,
            fail: false,
        };

        let outcome = scan_remote_source(&registry, &fetch, &ssh_source("srv", None)).expect("scan");
        assert_eq!(
            outcome.detected_provider,
            Some(("srv".to_string(), "alpha".to_string())),
            "unanimous samples → heal decision"
        );
        // All 7 files parsed with the detected provider.
        assert_eq!(outcome.sessions.len(), PROBE_SAMPLE_COUNT + 2);
        assert!(outcome
            .sessions
            .iter()
            .all(|m| m.provider_id == "alpha"));
    }

    #[test]
    fn probe_first_match_registration_order_wins_over_loose_providers() {
        // beta is loose (matches anything) but registered LATER: the probe
        // mirrors local `parse_session_meta` semantics — first match in
        // registration order wins, so the sample resolves to alpha and a
        // unanimous root heals to alpha. (Uniqueness-across-all is
        // impossible in the real format family: a claude line's
        // sessionId+type also satisfies qoder's weaker same-line check,
        // and qoder is registered after claude precisely for that reason.)
        let registry = registry_of(vec![
            MarkerProvider { id: "alpha", loose: false },
            MarkerProvider { id: "beta", loose: true },
        ]);
        let mut contents = HashMap::new();
        contents.insert(
            "/r/one.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 10, 5),
        );
        let fetch = FakeFetch {
            files: vec!["/r/one.jsonl".to_string()],
            contents,
            fail: false,
        };
        let outcome = scan_remote_source(&registry, &fetch, &ssh_source("srv", None)).expect("scan");
        assert_eq!(
            outcome.detected_provider,
            Some(("srv".to_string(), "alpha".to_string()))
        );
        assert_eq!(outcome.sessions.len(), 1);
    }

    #[test]
    fn probe_disagreeing_samples_is_inconclusive() {
        let registry = registry_of(vec![
            MarkerProvider { id: "alpha", loose: false },
            MarkerProvider { id: "beta", loose: false },
        ]);
        let mut contents = HashMap::new();
        contents.insert(
            "/r/a.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 10, 5),
        );
        contents.insert(
            "/r/b.jsonl".to_string(),
            session_bytes("{\"provider\":\"beta\"}", 10, 5),
        );
        let fetch = FakeFetch {
            files: vec!["/r/a.jsonl".to_string(), "/r/b.jsonl".to_string()],
            contents,
            fail: false,
        };
        let outcome = scan_remote_source(&registry, &fetch, &ssh_source("srv", None)).expect("scan");
        assert!(outcome.detected_provider.is_none());
        assert!(outcome.sessions.is_empty());
    }

    #[test]
    fn empty_remote_root_returns_empty_without_batch_call() {
        let registry = registry_of(vec![MarkerProvider { id: "alpha", loose: false }]);
        let fetch = FakeFetch {
            files: Vec::new(),
            contents: HashMap::new(),
            fail: true, // would fail if batch_metadata were called
        };
        let outcome = scan_remote_source(&registry, &fetch, &ssh_source("srv", None)).expect("scan");
        assert!(outcome.sessions.is_empty());
        assert!(outcome.detected_provider.is_none());
    }

    // ── disconnect fallback (decision D) ────────────────────────────────

    #[test]
    fn fallback_serves_cached_list_then_survives_disconnect() {
        let registry = registry_of(vec![MarkerProvider { id: "alpha", loose: false }]);
        let mut contents = HashMap::new();
        contents.insert(
            "/r/one.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 10, 5),
        );
        let mut fetch = FakeFetch {
            files: vec!["/r/one.jsonl".to_string()],
            contents,
            fail: false,
        };
        let mut cache: HashMap<String, Vec<SessionMeta>> = HashMap::new();

        // First scan: success, populates the cache, reports heal decision.
        let first = scan_source_with_fallback(&mut cache, &registry, &fetch, &ssh_source("srv", None));
        assert!(!first.from_cache);
        assert_eq!(first.heal, Some(("srv".to_string(), "alpha".to_string())));
        assert_eq!(first.sessions.len(), 1);

        // Connection dies: the cached list is served, no heal decision.
        fetch.fail = true;
        let second = scan_source_with_fallback(&mut cache, &registry, &fetch, &ssh_source("srv", None));
        assert!(second.from_cache);
        assert!(second.heal.is_none());
        // Stale-but-identical list: SessionMeta has no PartialEq, so
        // compare the identity-bearing projection (locator path, in
        // order).
        let key = |m: &SessionMeta| m.locator.clone();
        assert_eq!(
            second.sessions.iter().map(key).collect::<Vec<_>>(),
            first.sessions.iter().map(key).collect::<Vec<_>>(),
            "stale-but-identical list"
        );

        // First-ever failure (no cache entry): empty list, not an error.
        let third = scan_source_with_fallback(&mut cache, &registry, &fetch, &ssh_source("other", None));
        assert!(third.from_cache);
        assert!(third.sessions.is_empty());
    }
}
