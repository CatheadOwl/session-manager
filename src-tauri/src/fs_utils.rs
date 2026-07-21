use std::fs;
use std::path::{Path, PathBuf};

/// Walk a directory recursively, collecting all non-agent `.jsonl` files with their modification times.
/// Returns `(path, mtime_ms)` pairs. Agent sessions (files starting with `agent-`) are skipped.
pub fn walk_jsonl_files(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files);
    files
}

fn collect_jsonl_files(root: &Path, files: &mut Vec<(PathBuf, u64)>) {
    if !root.exists() {
        return;
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            // Skip agent sessions
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with("agent-"))
                .unwrap_or(false)
            {
                continue;
            }
            if let Ok(meta) = entry.metadata() {
                if let Ok(mtime) = meta.modified() {
                    let mtime_ms = mtime
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(0);
                    files.push((path, mtime_ms));
                }
            }
        }
    }
}
