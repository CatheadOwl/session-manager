//! Remote batch scan (phase 3): build a source's
//! session list from ONE discovery exec + ONE batch-metadata exec,
//! reusing the LOCAL provider parsers through a temp-file bridge
//! (the "batch" exit; the P1 real-alias benchmark: ~14 ms/file vs
//! ~710 ms per-file).
//!
//! ## Root derivation — "remote source = another machine"
//!
//! There is NO root field in the ssh settings entry and NO provider
//! probing (both were removed from the contract). The remote machine is
//! assumed to be isomorphic to the local one: every provider scans its
//! own standard roots there, exactly like the local scan core
//! (`scan.rs`):
//!
//! - each registry provider's `roots()` (LOCAL absolute paths, e.g.
//!   `C:\Users\u\.claude\projects`) is stripped of the local home
//!   prefix (`dirs::home_dir()`), with separators normalized to `/`,
//!   yielding a home-relative path (`.claude/projects`);
//! - the remote directory is `$HOME/<rel>` — expanded by the REMOTE
//!   shell, never locally (the remote user's home is unknown here);
//! - scope semantics are copied verbatim from the local scan:
//!   `roots()[0]` = active, `roots()[1]` = archived. Active scans only
//!   the active root; Archived scans only the archived root, and a
//!   provider with no archived root is skipped (same rule as local);
//! - roots NOT under the local home prefix (if any provider ever has
//!   one) are skipped with a debug log — they have no derivable remote
//!   counterpart;
//! - a file's provider is the provider OWNING the root it was found
//!   under (directory ownership, no content probing).
//!
//! ## Discovery shape (one exec for ALL roots of the scope)
//!
//! The batch discipline forbids per-root round-trips, so one exec
//! walks every derived root. Before each root's `find` output, the
//! script prints an attribution header:
//!
//! ```text
//! ROOT\t<provider_id>\t<$HOME/rel>
//! ```
//!
//! The parser switches the "current provider" on each `ROOT` line and
//! attributes every following path line to it. Collision risk is nil
//! in practice: `find` output lines are absolute paths (they start
//! with `/`), a `ROOT\t…` line never does.
//!
//! ## Layering
//!
//! This module is Tauri-free and makes no decisions for the command
//! layer: `scan_remote_source` returns plain sessions, the disconnect
//! fallback keeps the last successful list, and nothing here writes
//! settings or emits events.
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
//! (no N× read shape is allowed for it in the scan path).
//!
//! ## Provider support matrix (enforced by discovery shape)
//!
//! Discovery only collects `*.jsonl`, which satisfies the P0a matrix by
//! construction: gemini (`.json` chats) and opencode (sqlite / storage
//! directory tree) are never returned, so no per-provider branch is
//! needed here. (gemini's derived roots are `.json`-backed and simply
//! yield no files; opencode's roots are outside `~` on the scanning
//! machine and are skipped by the home-prefix rule.)
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
use crate::session_manager::types::{SessionLocator, SessionMeta, SessionScope};

// ---------------------------------------------------------------------------
// Root derivation: remote roots from provider roots()
// ---------------------------------------------------------------------------
// The derivation itself now lives in the shared `scan_roots` module:
// the remote line and the local extra-source overlay consume
// ONE function — strip the local home prefix from each provider's
// `roots()`, normalize to posix — instead of duplicating the map. The
// re-exports below keep this module's historical names (and its tests)
// byte-identical; the remote side joins `$HOME/<rel>` (expanded by the
// REMOTE shell), the local overlay joins `<extra_root>/<rel>`.

pub use crate::session_manager::scan_roots::DerivedRoot as RemoteRoot;
// The `home_relative_posix` re-export is consumed by this module's tests
// (via `use super::*`); the lib itself no longer calls it directly.
#[allow(unused_imports)]
pub use crate::session_manager::scan_roots::home_relative_posix;

pub use crate::session_manager::scan_roots::derive_scan_roots_with_home as derive_remote_roots_with_home;

// ---------------------------------------------------------------------------
// BatchFetch: the IO surface the scan core consumes
// ---------------------------------------------------------------------------

/// One discovered file: its remote path plus the provider that OWNS
/// the root it was found under (directory ownership — no content
/// probing).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DiscoveredFile {
    pub provider_id: String,
    pub path: RemotePath,
}

/// The two exec round-trips a remote scan needs, as a trait so tests can
/// install a fake (the transport itself is `RemoteSession`, wrapped by
/// [`SessionBatchFetch`]). Keeping this seam synchronous lets the whole
/// scan core (temp files + provider parsers, all blocking local IO) run
/// on the blocking pool.
pub trait BatchFetch {
    /// One exec: list `*.jsonl` files under ALL of `roots` (the
    /// attribution wire shape lives in [`build_find_command`] /
    /// [`parse_find_output`]). Returned files are attributed to their
    /// root's provider.
    fn list_jsonl_files(&self, roots: &[RemoteRoot]) -> Result<Vec<DiscoveredFile>, RemoteError>;
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
    fn list_jsonl_files(&self, roots: &[RemoteRoot]) -> Result<Vec<DiscoveredFile>, RemoteError> {
        let script = build_find_command(roots);
        let stdout = self.handle.block_on(self.session.exec_script(&script))?;
        Ok(parse_find_output(&stdout))
    }

    fn batch_metadata(&self, files: &[RemotePath]) -> Result<Vec<FileMetadataBlob>, RemoteError> {
        self.handle.block_on(self.session.batch_metadata(files))
    }
}

// ---------------------------------------------------------------------------
// File discovery (one exec for all roots)
// ---------------------------------------------------------------------------

/// The `find` argument for one derived root: `"$HOME"/'<rel>'` — the
/// REMOTE shell expands `$HOME` (the remote user's home); the
/// home-relative part is single-quoted so spaces/globs stay literal.
fn root_find_arg(rel: &str) -> String {
    if rel.is_empty() {
        "\"$HOME\"".to_string()
    } else {
        format!("\"$HOME\"/{}", shell_quote(rel))
    }
}

/// Human-readable label for one derived root (`$HOME/<rel>`), echoed in
/// the ROOT attribution header for diagnostics.
fn root_label(rel: &str) -> String {
    if rel.is_empty() {
        "$HOME".to_string()
    } else {
        format!("$HOME/{rel}")
    }
}

/// Build the ONE discovery exec covering every root of the scope
/// (batch discipline: no per-root round-trips). Per root, in
/// order:
///
/// ```sh
/// printf 'ROOT\t%s\t%s\n' '<provider_id>' '<$HOME/rel>';
/// find "$HOME"/'<rel>' -type f -name '*.jsonl' 2>/dev/null
/// ```
///
/// The `ROOT` line attributes every following path line to that root's
/// provider (see [`parse_find_output`]). stderr is silenced per root:
/// an unreadable remote directory must not fail the whole scan.
pub fn build_find_command(roots: &[RemoteRoot]) -> String {
    roots
        .iter()
        .map(|r| {
            // `|| true` per find: a MISSING root dir makes find exit 1 —
            // that root simply has no sessions and must not fail the whole
            // discovery exec (stderr is already silenced for unreadables).
            format!(
                "printf 'ROOT\\t%s\\t%s\\n' {} {}; find {} -type f -name '*.jsonl' 2>/dev/null || true",
                shell_quote(&r.provider_id),
                shell_quote(&root_label(&r.rel)),
                root_find_arg(&r.rel),
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Parse discovery output into provider-attributed paths. A line of
/// the shape `ROOT\t<provider_id>\t<label>` switches the current
/// provider; every other non-empty line (CRLF tolerated) is a path
/// attributed to it. Path lines before the first `ROOT` header are
/// dropped (cannot happen with [`build_find_command`], defensive). The
/// batch protocol requires UTF-8 paths; non-UTF-8 output is lossily
/// converted rather than failing the whole scan (such paths fail batch
/// framing later and get skipped there).
pub fn parse_find_output(bytes: &[u8]) -> Vec<DiscoveredFile> {
    let mut files = Vec::new();
    let mut current: Option<String> = None;
    for line in String::from_utf8_lossy(bytes).lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '\t');
        if let ("ROOT", Some(provider_id), Some(_label)) =
            (parts.next().unwrap_or(""), parts.next(), parts.next())
        {
            current = Some(provider_id.to_string());
            continue;
        }
        if let Some(provider_id) = &current {
            files.push(DiscoveredFile {
                provider_id: provider_id.clone(),
                path: line.to_string(),
            });
        }
    }
    files
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

/// Scan one remote source for `scope`: derive the roots from the
/// registry (`roots()`, home-prefix strip — no settings involvement) →
/// ONE discovery exec over all roots → ONE batch-metadata exec →
/// temp-file bridge → each file parsed by its root's OWN provider.
/// Files that fail to parse are skipped, matching local scan behavior.
pub fn scan_remote_source(
    registry: &ProviderRegistry,
    fetch: &dyn BatchFetch,
    source: &SshSource,
    scope: &SessionScope,
) -> Result<Vec<SessionMeta>, RemoteError> {
    scan_remote_source_with_home(registry, fetch, source, scope, &crate::config::get_home_dir())
}

/// Test seam / core of [`scan_remote_source`] with an explicit home
/// prefix for root derivation (see [`derive_remote_roots_with_home`]).
pub fn scan_remote_source_with_home(
    registry: &ProviderRegistry,
    fetch: &dyn BatchFetch,
    source: &SshSource,
    scope: &SessionScope,
    home: &Path,
) -> Result<Vec<SessionMeta>, RemoteError> {
    let roots = derive_remote_roots_with_home(registry, scope, home);
    let mut files = fetch.list_jsonl_files(&roots)?;
    if files.is_empty() {
        return Ok(Vec::new());
    }
    // Deterministic order: stable output lists.
    files.sort();

    let paths: Vec<RemotePath> = files.iter().map(|f| f.path.clone()).collect();
    let blobs = fetch.batch_metadata(&paths)?;

    // Path-keyed join, NOT a positional zip: the batch
    // stream drops MISS lines (files deleted between discovery and
    // batch), so a zip would shift every post-miss blob onto the wrong
    // file (wrong provider parse) and truncate the tail. Blobs carry
    // their own `path` — the discovery path echoed back by the remote
    // script verbatim — so exact-match pairing is free.
    let blobs_by_path: HashMap<&str, &FileMetadataBlob> =
        blobs.iter().map(|b| (b.path.as_str(), b)).collect();
    // A pairing shortfall is also the visible signature of a remote
    // userland the script cannot stat (the macOS BSD-stat failure:
    // every file MISS → 0 blobs → previously a silent "0 sessions"
    // success). Warn, never silently degrade to "no sessions".
    let paired = files
        .iter()
        .filter(|f| blobs_by_path.contains_key(f.path.as_str()))
        .count();
    if paired != files.len() {
        log::warn!(
            "remote scan: paired {paired}/{} discovered files with batch \
             metadata — unpaired files skipped (vanished remotely, or a \
             stat/userland mismatch on the remote host)",
            files.len()
        );
    }

    let temp = tempfile::tempdir()
        .map_err(|e| RemoteError::Io(format!("scan tempdir: {e}")))?;
    let mut sessions = Vec::new();
    for (idx, file) in files.iter().enumerate() {
        // Unpaired (MISS) files are skipped — surfaced in the pairing
        // warn above.
        let Some(&blob) = blobs_by_path.get(file.path.as_str()) else {
            continue;
        };
        // Directory ownership: the provider that owns the root the file
        // was found under owns the parse. Registry lookup cannot fail —
        // the id came from the registry itself in derive_remote_roots.
        let provider = registry
            .get(&file.provider_id)
            .expect("provider id originated from the registry");
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
        // Display normalization: prefix project_dir with the source id so
        // folder grouping (whole-string equality on the normalized dir)
        // never merges the same-named project across machines, and the
        // group name tells the user WHICH machine it belongs to
        // (`ali:/home/admin/projects/GaaS_meta`). Keyed by the entry id,
        // not the auth alias or label: the id is the stable, unique anchor
        // shared by both auth modes (sshConfig and manual). Grouping never
        // parses the prefix, so ids containing ':' are unambiguous. The
        // provider-parsed cwd itself is NOT rewritten — this is the same
        // layer and rationale as the locator re-anchoring above.
        if let Some(dir) = meta.project_dir.take() {
            meta.project_dir = Some(format!("{}:{}", source.id, dir));
        }
        sessions.push(meta);
    }
    // The tempdir (and every bridge file) is dropped here — scratch by
    // construction, cleaned even on early returns via tempdir's Drop.
    Ok(sessions)
}

// ---------------------------------------------------------------------------
// Disconnect fallback wrapper
// ---------------------------------------------------------------------------

/// Per-source scan result handed to the command layer: the session list
/// to append (cached list on failure).
#[derive(Debug, Clone)]
pub struct RemoteSourceResult {
    /// Sessions to append to the list result (cached list on failure).
    pub sessions: Vec<SessionMeta>,
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
            from_cache: true,
        }
    }
}

/// Scan one source with the v1 disconnect semantics: on success, refresh
/// `last_scan`; on failure, serve the cached list (empty when the very
/// first scan failed) with a warn — a dead remote source must never
/// block or empty the local list.
pub fn scan_source_with_fallback(
    last_scan: &mut HashMap<String, Vec<SessionMeta>>,
    registry: &ProviderRegistry,
    fetch: &dyn BatchFetch,
    source: &SshSource,
    scope: &SessionScope,
) -> RemoteSourceResult {
    match scan_remote_source(registry, fetch, source, scope) {
        Ok(sessions) => {
            last_scan.insert(source.id.clone(), sessions.clone());
            RemoteSourceResult {
                sessions,
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
    use std::cell::RefCell;
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

    fn ssh_source(id: &str) -> SshSource {
        SshSource {
            id: id.to_string(),
            label: None,
            host: "h".to_string(),
            port: 22,
            user: "u".to_string(),
            auth: crate::session_manager::settings::SourceAuth::Agent,
            enabled: true,
            extra: std::collections::BTreeMap::new(),
        }
    }

    /// Fake transport: canned attributed file list + contents,
    /// switchable failure, a recorder for the roots each discovery
    /// call received (to pin the scope plumbing), and a `reverse_blobs`
    /// switch to deliver metadata out of request order (the transport
    /// makes no order promise — pairing must be path-keyed).
    struct FakeFetch {
        files: Vec<DiscoveredFile>,
        contents: HashMap<RemotePath, Vec<u8>>,
        fail: bool,
        reverse_blobs: bool,
        seen_roots: RefCell<Vec<Vec<RemoteRoot>>>,
    }

    impl FakeFetch {
        fn new(files: Vec<DiscoveredFile>, contents: HashMap<RemotePath, Vec<u8>>) -> Self {
            Self {
                files,
                contents,
                fail: false,
                reverse_blobs: false,
                seen_roots: RefCell::new(Vec::new()),
            }
        }
    }

    impl BatchFetch for FakeFetch {
        fn list_jsonl_files(&self, roots: &[RemoteRoot]) -> Result<Vec<DiscoveredFile>, RemoteError> {
            self.seen_roots.borrow_mut().push(roots.to_vec());
            Ok(self.files.clone())
        }
        fn batch_metadata(&self, files: &[RemotePath]) -> Result<Vec<FileMetadataBlob>, RemoteError> {
            if self.fail {
                return Err(RemoteError::Disconnected);
            }
            // Files absent from `contents` yield no blob — the remote
            // MISS shape (`parse_batch_stream` drops MISS lines), which
            // the path-pairing tests rely on.
            let mut blobs: Vec<FileMetadataBlob> = files
                .iter()
                .filter_map(|f| self.contents.get(f).map(|c| blob_for(f, c)))
                .collect();
            if self.reverse_blobs {
                blobs.reverse();
            }
            Ok(blobs)
        }
    }

    /// Marker-based fixture provider: parses a file iff its first line
    /// contains `"provider":"<id>"`. Roots are injected so root-derivation
    /// tests control the home layout.
    struct MarkerProvider {
        id: &'static str,
        roots: Vec<PathBuf>,
    }

    impl SessionProvider for MarkerProvider {
        fn id(&self) -> &str {
            self.id
        }
        fn roots(&self) -> Vec<PathBuf> {
            self.roots.clone()
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
            if !first.contains(&marker) {
                return None;
            }
            // Optional `"project":"<dir>"` marker exercises the project_dir
            // plumbing (source-id prefix) without a real provider format.
            let project_dir = first
                .split("\"project\":\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .map(str::to_string);
            let session_id = path.file_stem()?.to_str()?.to_string();
            let file = path.to_string_lossy().into_owned();
            Some(SessionMeta {
                provider_id: self.id.to_string(),
                session_id,
                title: Some(self.id.to_string()),
                summary: None,
                project_dir,
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

    // ── root derivation ───────────────────────────────────────────────

    #[test]
    fn derive_roots_strips_home_and_normalizes_separators() {
        // Fake home: the tempdir stands in for the real home prefix, so
        // the test is OS-independent. On Windows the provider roots are
        // built with native backslashes via PathBuf::join — the derived
        // rel must come out posix-normalized either way.
        let home = tempdir().expect("tempdir");
        let registry = registry_of(vec![MarkerProvider {
            id: "alpha",
            roots: vec![
                home.path().join(".alpha").join("projects"),
                home.path().join(".alpha").join("archived"),
            ],
        }]);

        let active = derive_remote_roots_with_home(&registry, &SessionScope::Active, home.path());
        assert_eq!(
            active,
            vec![RemoteRoot {
                provider_id: "alpha".to_string(),
                rel: ".alpha/projects".to_string(),
            }],
            "active = roots()[0], home-stripped, '/'-separated"
        );

        let archived = derive_remote_roots_with_home(&registry, &SessionScope::Archived, home.path());
        assert_eq!(
            archived,
            vec![RemoteRoot {
                provider_id: "alpha".to_string(),
                rel: ".alpha/archived".to_string(),
            }],
            "archived = roots()[1]"
        );
    }

    #[test]
    fn derive_roots_archived_skips_providers_without_archive_root() {
        let home = tempdir().expect("tempdir");
        let registry = registry_of(vec![
            MarkerProvider {
                id: "one-root",
                roots: vec![home.path().join(".one")],
            },
            MarkerProvider {
                id: "two-root",
                roots: vec![home.path().join(".two"), home.path().join(".two-arch")],
            },
        ]);
        let archived = derive_remote_roots_with_home(&registry, &SessionScope::Archived, home.path());
        assert_eq!(
            archived,
            vec![RemoteRoot {
                provider_id: "two-root".to_string(),
                rel: ".two-arch".to_string(),
            }],
            "no archived root → provider skipped (local parity)"
        );
    }

    #[test]
    fn derive_roots_skips_roots_outside_home() {
        let home = tempdir().expect("tempdir");
        let outside = tempdir().expect("tempdir");
        let registry = registry_of(vec![MarkerProvider {
            id: "alpha",
            roots: vec![outside.path().join("data")],
        }]);
        // Sibling directories must NOT match the home prefix
        // (component-wise strip, not a string prefix).
        assert!(derive_remote_roots_with_home(&registry, &SessionScope::Active, home.path()).is_empty());
    }

    #[test]
    fn home_relative_root_itself_is_empty_rel() {
        let home = tempdir().expect("tempdir");
        assert_eq!(
            home_relative_posix(home.path(), home.path()),
            Some(String::new()),
            "root == home → empty rel → the remote dir is $HOME itself"
        );
    }

    // ── discovery command + attribution parsing ─────────────────────────

    #[test]
    fn find_command_covers_all_roots_with_attribution_headers() {
        let roots = vec![
            RemoteRoot {
                provider_id: "claude".to_string(),
                rel: ".claude/projects".to_string(),
            },
            RemoteRoot {
                provider_id: "codex".to_string(),
                rel: ".codex/sessions".to_string(),
            },
        ];
        assert_eq!(
            build_find_command(&roots),
            "printf 'ROOT\\t%s\\t%s\\n' 'claude' '$HOME/.claude/projects'; \
             find \"$HOME\"/'.claude/projects' -type f -name '*.jsonl' 2>/dev/null || true; \
             printf 'ROOT\\t%s\\t%s\\n' 'codex' '$HOME/.codex/sessions'; \
             find \"$HOME\"/'.codex/sessions' -type f -name '*.jsonl' 2>/dev/null || true"
        );
    }

    #[test]
    fn find_command_home_root_and_special_characters_stay_quoted() {
        // rel == "" (a provider whose root IS home) → bare $HOME.
        assert_eq!(
            build_find_command(&[RemoteRoot {
                provider_id: "odd".to_string(),
                rel: String::new(),
            }]),
            "printf 'ROOT\\t%s\\t%s\\n' 'odd' '$HOME'; \
             find \"$HOME\" -type f -name '*.jsonl' 2>/dev/null || true"
        );
        // Provider ids / rels with shell metacharacters stay literal.
        assert!(build_find_command(&[RemoteRoot {
            provider_id: "a'b".to_string(),
            rel: "my sessions".to_string(),
        }])
        .contains("'a'\\''b' '$HOME/my sessions'"));
    }

    #[test]
    fn find_output_attributes_paths_to_the_last_root_header() {
        let out = parse_find_output(
            b"ROOT\tclaude\t$HOME/.claude/projects\n\
              /home/u/.claude/projects/p/one.jsonl\r\n\
              /home/u/.claude/projects/p/two.jsonl\n\
              ROOT\tcodex\t$HOME/.codex/sessions\n\
              /home/u/.codex/sessions/three.jsonl\n",
        );
        assert_eq!(
            out,
            vec![
                DiscoveredFile {
                    provider_id: "claude".to_string(),
                    path: "/home/u/.claude/projects/p/one.jsonl".to_string(),
                },
                DiscoveredFile {
                    provider_id: "claude".to_string(),
                    path: "/home/u/.claude/projects/p/two.jsonl".to_string(),
                },
                DiscoveredFile {
                    provider_id: "codex".to_string(),
                    path: "/home/u/.codex/sessions/three.jsonl".to_string(),
                },
            ]
        );
    }

    #[test]
    fn find_output_drops_blanks_and_unheaded_paths() {
        assert!(parse_find_output(b"").is_empty());
        // A path line before any ROOT header has no owner → dropped
        // (defensive; build_find_command always emits a header first).
        assert!(parse_find_output(b"/orphan.jsonl\n").is_empty());
        assert_eq!(
            parse_find_output(b"ROOT\tp\t$HOME/p\n\n/p/a.jsonl\n"),
            vec![DiscoveredFile {
                provider_id: "p".to_string(),
                path: "/p/a.jsonl".to_string(),
            }]
        );
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

    // ── scan core: per-root provider ownership, locators, skip semantics

    #[test]
    fn scan_prefixes_project_dir_with_source_id_and_keeps_none() {
        // Display normalization: remote project_dir gains the source-id
        // prefix so the same-named project on two machines never merges in
        // folder grouping (whole-string equality); None stays None. The
        // same content scanned under two source ids yields two distinct
        // dirs — grouping isolation by construction.
        let home = tempdir().expect("tempdir");
        let registry = registry_of(vec![MarkerProvider {
            id: "alpha",
            roots: vec![home.path().join(".alpha")],
        }]);
        let mut contents = HashMap::new();
        contents.insert(
            "/r/proj.jsonl".to_string(),
            session_bytes(
                "{\"provider\":\"alpha\",\"project\":\"/home/admin/projects/GaaS_meta\"}",
                20,
                5,
            ),
        );
        contents.insert(
            "/r/bare.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 20, 5),
        );
        let fetch = FakeFetch::new(
            vec![
                DiscoveredFile {
                    provider_id: "alpha".to_string(),
                    path: "/r/proj.jsonl".to_string(),
                },
                DiscoveredFile {
                    provider_id: "alpha".to_string(),
                    path: "/r/bare.jsonl".to_string(),
                },
            ],
            contents,
        );

        let find = |id: &str| {
            scan_remote_source_with_home(
                &registry,
                &fetch,
                &ssh_source(id),
                &SessionScope::Active,
                home.path(),
            )
            .expect("scan")
            .into_iter()
            .map(|m| m.project_dir)
            .collect::<Vec<_>>()
        };

        let mut ali = find("ali");
        ali.sort();
        assert_eq!(
            ali,
            vec![
                None,
                Some("ali:/home/admin/projects/GaaS_meta".to_string())
            ]
        );
        let office = find("office");
        assert_eq!(
            office,
            vec![
                None,
                Some("office:/home/admin/projects/GaaS_meta".to_string())
            ]
        );
    }

    #[test]
    fn scan_routes_each_file_to_its_roots_provider_and_anchors_remote_locators() {
        let home = tempdir().expect("tempdir");
        let registry = registry_of(vec![
            MarkerProvider {
                id: "alpha",
                roots: vec![home.path().join(".alpha")],
            },
            MarkerProvider {
                id: "beta",
                roots: vec![home.path().join(".beta")],
            },
        ]);
        let mut contents = HashMap::new();
        contents.insert(
            "/r/one.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 20, 5),
        );
        // alpha's parser cannot parse this one (beta marker) → skipped,
        // matching local scan parity: the ROOT OWNS the file, content
        // never overrides ownership.
        contents.insert(
            "/r/broken.jsonl".to_string(),
            session_bytes("{\"provider\":\"beta\"}", 20, 5),
        );
        contents.insert(
            "/r/two.jsonl".to_string(),
            session_bytes("{\"provider\":\"beta\"}", 500, 60),
        );
        let fetch = FakeFetch::new(
            vec![
                DiscoveredFile {
                    provider_id: "alpha".to_string(),
                    path: "/r/one.jsonl".to_string(),
                },
                DiscoveredFile {
                    provider_id: "alpha".to_string(),
                    path: "/r/broken.jsonl".to_string(),
                },
                DiscoveredFile {
                    provider_id: "beta".to_string(),
                    path: "/r/two.jsonl".to_string(),
                },
            ],
            contents,
        );

        let sessions = scan_remote_source_with_home(
            &registry,
            &fetch,
            &ssh_source("srv"),
            &SessionScope::Active,
            home.path(),
        )
        .expect("scan");
        let ids: Vec<&str> = sessions.iter().map(|m| m.session_id.as_str()).collect();
        assert_eq!(ids, vec!["one", "two"], "unparseable-for-owner file skipped");
        for meta in &sessions {
            assert_eq!(meta.source_path, None, "source_path stays a LOCAL-path concept");
            match &meta.locator {
                Some(SessionLocator::Remote { source_id, path }) => {
                    assert_eq!(source_id, "srv");
                    assert!(path.starts_with("/r/"));
                }
                other => panic!("expected Remote locator, got {other:?}"),
            }
        }
        // Ownership: each parsed file was parsed by its ROOT's provider.
        assert_eq!(sessions[0].provider_id, "alpha");
        assert_eq!(sessions[1].provider_id, "beta");
        // The scope flowed into the discovery call: the derived roots
        // (active roots of both providers) are what the fetch saw.
        let seen = fetch.seen_roots.borrow().clone();
        assert_eq!(
            seen,
            &[vec![
                RemoteRoot {
                    provider_id: "alpha".to_string(),
                    rel: ".alpha".to_string(),
                },
                RemoteRoot {
                    provider_id: "beta".to_string(),
                    rel: ".beta".to_string(),
                },
            ]],
            "discovery received the scope-derived roots"
        );
    }

    #[test]
    fn empty_discovery_returns_empty_without_batch_call() {
        let home = tempdir().expect("tempdir");
        let registry = registry_of(vec![MarkerProvider {
            id: "alpha",
            roots: vec![home.path().join(".alpha")],
        }]);
        let fetch = FakeFetch {
            files: Vec::new(),
            contents: HashMap::new(),
            fail: true, // would fail if batch_metadata were called
            reverse_blobs: false,
            seen_roots: RefCell::new(Vec::new()),
        };
        let sessions = scan_remote_source_with_home(
            &registry,
            &fetch,
            &ssh_source("srv"),
            &SessionScope::Active,
            home.path(),
        )
        .expect("scan");
        assert!(sessions.is_empty());
    }

    // ── path-keyed blob pairing (no positional zip) ─────────────────

    /// A blob set missing one file (the MISS shape: the remote deleted
    /// it between discovery and batch) must skip exactly that file —
    /// not shift the remaining blobs onto the wrong files and not
    /// truncate the tail, which is what a positional zip did.
    #[test]
    fn scan_pairs_blobs_by_path_and_skips_only_unpaired_files() {
        let home = tempdir().expect("tempdir");
        let registry = registry_of(vec![MarkerProvider {
            id: "alpha",
            roots: vec![home.path().join(".alpha")],
        }]);
        let mut contents = HashMap::new();
        contents.insert(
            "/r/one.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 20, 5),
        );
        // /r/two.jsonl: discovered, no blob (MISS).
        contents.insert(
            "/r/three.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 20, 5),
        );
        let fetch = FakeFetch::new(
            vec![
                DiscoveredFile {
                    provider_id: "alpha".to_string(),
                    path: "/r/one.jsonl".to_string(),
                },
                DiscoveredFile {
                    provider_id: "alpha".to_string(),
                    path: "/r/two.jsonl".to_string(),
                },
                DiscoveredFile {
                    provider_id: "alpha".to_string(),
                    path: "/r/three.jsonl".to_string(),
                },
            ],
            contents,
        );

        let sessions = scan_remote_source_with_home(
            &registry,
            &fetch,
            &ssh_source("srv"),
            &SessionScope::Active,
            home.path(),
        )
        .expect("scan");
        let ids: Vec<&str> = sessions.iter().map(|m| m.session_id.as_str()).collect();
        assert_eq!(ids, vec!["one", "three"], "only the unpaired file is skipped");
    }

    /// The transport makes no order promise: with blobs arriving
    /// reversed relative to the sorted discovery list, a positional zip
    /// would hand every file the WRONG provider's parser (alpha file
    /// parsed as beta → parse fails → silently dropped). Path pairing
    /// must attribute content to its own file regardless of blob order.
    #[test]
    fn scan_out_of_order_blobs_pair_by_path_not_position() {
        let home = tempdir().expect("tempdir");
        let registry = registry_of(vec![
            MarkerProvider {
                id: "alpha",
                roots: vec![home.path().join(".alpha")],
            },
            MarkerProvider {
                id: "beta",
                roots: vec![home.path().join(".beta")],
            },
        ]);
        let mut contents = HashMap::new();
        contents.insert(
            "/r/one.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 20, 5),
        );
        contents.insert(
            "/r/two.jsonl".to_string(),
            session_bytes("{\"provider\":\"beta\"}", 20, 5),
        );
        let mut fetch = FakeFetch::new(
            vec![
                DiscoveredFile {
                    provider_id: "alpha".to_string(),
                    path: "/r/one.jsonl".to_string(),
                },
                DiscoveredFile {
                    provider_id: "beta".to_string(),
                    path: "/r/two.jsonl".to_string(),
                },
            ],
            contents,
        );
        fetch.reverse_blobs = true;

        let sessions = scan_remote_source_with_home(
            &registry,
            &fetch,
            &ssh_source("srv"),
            &SessionScope::Active,
            home.path(),
        )
        .expect("scan");
        let ids: Vec<(&str, &str)> = sessions
            .iter()
            .map(|m| (m.provider_id.as_str(), m.session_id.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec![("alpha", "one"), ("beta", "two")],
            "each file parsed by its own provider despite reversed blobs"
        );
    }

    // ── disconnect fallback (decision D) ────────────────────────────────

    #[test]
    fn fallback_serves_cached_list_then_survives_disconnect() {
        let home = tempdir().expect("tempdir");
        let registry = registry_of(vec![MarkerProvider {
            id: "alpha",
            roots: vec![home.path().join(".alpha")],
        }]);
        let mut contents = HashMap::new();
        contents.insert(
            "/r/one.jsonl".to_string(),
            session_bytes("{\"provider\":\"alpha\"}", 10, 5),
        );
        let mut fetch = FakeFetch::new(
            vec![DiscoveredFile {
                provider_id: "alpha".to_string(),
                path: "/r/one.jsonl".to_string(),
            }],
            contents,
        );
        let mut cache: HashMap<String, Vec<SessionMeta>> = HashMap::new();

        // First scan: success, populates the cache.
        let first = scan_source_with_fallback(
            &mut cache,
            &registry,
            &fetch,
            &ssh_source("srv"),
            &SessionScope::Active,
        );
        assert!(!first.from_cache);
        assert_eq!(first.sessions.len(), 1);

        // Connection dies: the cached list is served.
        fetch.fail = true;
        let second = scan_source_with_fallback(
            &mut cache,
            &registry,
            &fetch,
            &ssh_source("srv"),
            &SessionScope::Active,
        );
        assert!(second.from_cache);
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
        let third = scan_source_with_fallback(
            &mut cache,
            &registry,
            &fetch,
            &ssh_source("other"),
            &SessionScope::Active,
        );
        assert!(third.from_cache);
        assert!(third.sessions.is_empty());
    }
}
