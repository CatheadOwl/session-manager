//! Transient local cache for fully-fetched remote files (ADR 0007
//! "cache" exit: opening a session pays the full transfer once, then
//! mtime+size-gated reuse makes re-opens free).
//!
//! Layout (an implementation detail, not a contract — safe to wipe):
//!
//! ```text
//! <cache_dir>/session-manager/remote-cache/<sha256(source_id)>/<sha256(path)>
//! <same>.meta.json     {"size": ..., "mtime": ...}
//! <same>.part          exists only mid-write (side-write + rename)
//! ```
//!
//! Both key components are hashed, never interpolated verbatim: a
//! source id or remote path containing separators/quotes cannot escape
//! its cache directory. Writes are atomic by construction — the payload
//! lands under a `.part` sibling and is renamed into place only when
//! complete, so a cancelled fetch (user closes the detail view mid-
//! transfer) can at worst leave a `.part` file behind, never a torn
//! cache entry that a later freshness check would accept.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Freshness pair for one cached remote file.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheMeta {
    pub size: u64,
    /// mtime in whole seconds since the Unix epoch (`stat -c %Y` /
    /// SFTP attrs agree on this unit).
    pub mtime: i64,
}

/// Default cache root: `<OS cache dir>/session-manager/remote-cache`.
/// Falls back to the temp dir if the platform has no cache dir.
pub fn default_cache_base() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("session-manager")
        .join("remote-cache")
}

fn hash_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Cache entry path for (source_id, remote_path). Pure function of its
/// inputs — the same pair always maps to the same entry.
pub fn entry_path(base: &Path, source_id: &str, remote_path: &str) -> PathBuf {
    base.join(hash_hex(source_id.as_bytes()))
        .join(hash_hex(remote_path.as_bytes()))
}

/// Sidecar meta path for an entry (`<entry>.meta.json`).
pub fn meta_path(entry: &Path) -> PathBuf {
    let name = entry
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    entry.with_file_name(format!("{name}.meta.json"))
}

/// Read the sidecar meta, if present and well-formed.
pub fn read_meta(base: &Path, source_id: &str, remote_path: &str) -> Option<CacheMeta> {
    let entry = entry_path(base, source_id, remote_path);
    let text = fs::read_to_string(meta_path(&entry)).ok()?;
    serde_json::from_str(&text).ok()
}

/// True when the cache entry exists AND its recorded (size, mtime)
/// matches the given attrs — the gate `fetch_to_local` consults before
/// paying a transfer.
pub fn is_fresh(
    base: &Path,
    source_id: &str,
    remote_path: &str,
    size: u64,
    mtime: i64,
) -> bool {
    read_meta(base, source_id, remote_path).is_some_and(|meta| {
        meta == CacheMeta { size, mtime } && entry_path(base, source_id, remote_path).is_file()
    })
}

/// Atomically store a fully-fetched file plus its sidecar meta.
/// Returns the final cache entry path.
pub fn store(
    base: &Path,
    source_id: &str,
    remote_path: &str,
    bytes: &[u8],
    meta: CacheMeta,
) -> io::Result<PathBuf> {
    let entry = entry_path(base, source_id, remote_path);
    if let Some(parent) = entry.parent() {
        fs::create_dir_all(parent)?;
    }
    // Side-write + rename: a reader either sees the old entry, the new
    // complete entry, or (briefly) no entry — never a partial one.
    let part = entry.with_file_name(format!(
        "{}.part",
        entry.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    ));
    fs::write(&part, bytes)?;
    fs::rename(&part, &entry)?;
    fs::write(meta_path(&entry), serde_json::to_string(&meta).unwrap_or_default())?;
    log::debug!(
        "remote-cache: stored {} ({} bytes, size={}, mtime={})",
        entry.display(),
        bytes.len(),
        meta.size,
        meta.mtime
    );
    Ok(entry)
}

/// Remove one cache entry (payload + sidecar). Best-effort: a missing
/// entry or a locked file is fine (transient cache, next fetch
/// overwrites). A stale `.part` sibling is swept too.
pub fn invalidate(base: &Path, source_id: &str, remote_path: &str) {
    let entry = entry_path(base, source_id, remote_path);
    let name = entry
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let part = entry.with_file_name(format!("{name}.part"));
    let _ = fs::remove_file(&part);
    let _ = fs::remove_file(meta_path(&entry));
    let removed = fs::remove_file(&entry).is_ok();
    log::debug!(
        "remote-cache: invalidated {} (removed={removed})",
        entry.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_path_is_pure_and_separator_safe() {
        let base = Path::new("/cache");
        let a = entry_path(base, "src1", "/remote/file.jsonl");
        let b = entry_path(base, "src1", "/remote/file.jsonl");
        let c = entry_path(base, "src2", "/remote/file.jsonl");
        let d = entry_path(base, "src1", "/remote/other.jsonl");
        assert_eq!(a, b, "same inputs → same entry");
        assert_ne!(a, c, "different source → different entry");
        assert_ne!(a, d, "different path → different entry");
        // Hashed components: hostile ids cannot traverse out of base.
        let evil = entry_path(base, "../../etc", "..\\..\\c:\\x");
        assert!(evil.starts_with(base), "{}", evil.display());
        assert_eq!(evil.components().count(), base.components().count() + 2);
        // Hash output is hex-only.
        for comp in evil.components().skip(base.components().count()) {
            let s = comp.as_os_str().to_string_lossy();
            assert!(s.chars().all(|c| c.is_ascii_hexdigit()), "{s}");
        }
    }

    #[test]
    fn meta_path_is_sidecar_of_entry() {
        let entry = Path::new("/cache/aa/bb");
        assert_eq!(meta_path(entry), Path::new("/cache/aa/bb.meta.json"));
    }

    #[test]
    fn store_then_is_fresh_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        store(base, "s1", "/r/a.jsonl", b"payload", CacheMeta { size: 7, mtime: 9 })
            .expect("store");
        assert!(is_fresh(base, "s1", "/r/a.jsonl", 7, 9));
        // Any attr change breaks freshness.
        assert!(!is_fresh(base, "s1", "/r/a.jsonl", 8, 9));
        assert!(!is_fresh(base, "s1", "/r/a.jsonl", 7, 10));
        // Unknown entries are not fresh.
        assert!(!is_fresh(base, "s1", "/r/missing.jsonl", 1, 1));
        // The stored payload is readable at the returned entry path.
        let entry = entry_path(base, "s1", "/r/a.jsonl");
        assert_eq!(fs::read(&entry).expect("read"), b"payload");
    }

    #[test]
    fn invalidate_removes_entry_and_meta() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        store(base, "s1", "/r/a.jsonl", b"payload", CacheMeta { size: 7, mtime: 9 })
            .expect("store");
        let entry = entry_path(base, "s1", "/r/a.jsonl");
        assert!(entry.is_file());
        assert!(meta_path(&entry).is_file());
        invalidate(base, "s1", "/r/a.jsonl");
        assert!(!entry.exists());
        assert!(!meta_path(&entry).exists());
        // Idempotent on a missing entry.
        invalidate(base, "s1", "/r/a.jsonl");
    }

    #[test]
    fn invalidate_sweeps_stale_part_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let entry = entry_path(base, "s1", "/r/a.jsonl");
        fs::create_dir_all(entry.parent().unwrap()).expect("dirs");
        let name = entry.file_name().unwrap().to_string_lossy().into_owned();
        let part = entry.with_file_name(format!("{name}.part"));
        fs::write(&part, b"half-written").expect("part");
        invalidate(base, "s1", "/r/a.jsonl");
        assert!(!part.exists(), "stale .part swept by invalidate");
    }

    #[test]
    fn store_overwrites_previous_entry_atomically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        store(base, "s1", "/r/a.jsonl", b"old", CacheMeta { size: 3, mtime: 1 })
            .expect("store old");
        store(base, "s1", "/r/a.jsonl", b"new-longer", CacheMeta { size: 10, mtime: 2 })
            .expect("store new");
        let entry = entry_path(base, "s1", "/r/a.jsonl");
        assert_eq!(fs::read(&entry).expect("read"), b"new-longer");
        assert!(is_fresh(base, "s1", "/r/a.jsonl", 10, 2));
        assert!(!is_fresh(base, "s1", "/r/a.jsonl", 3, 1));
        // No .part residue after a clean store.
        let name = entry.file_name().unwrap().to_string_lossy().into_owned();
        assert!(!entry.with_file_name(format!("{name}.part")).exists());
    }

    #[test]
    fn read_meta_tolerates_garbage_sidecar() {
        let dir = tempfile::tempdir().expect("tempdir");
        let base = dir.path();
        let entry = entry_path(base, "s1", "/r/a.jsonl");
        fs::create_dir_all(entry.parent().unwrap()).expect("dirs");
        fs::write(meta_path(&entry), "not json").expect("garbage");
        assert_eq!(read_meta(base, "s1", "/r/a.jsonl"), None);
    }
}
