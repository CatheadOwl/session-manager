use std::fs;
use std::path::{Path, PathBuf};

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

/// Walk a directory recursively, collecting all non-agent `.jsonl` files with their modification times.
/// Returns `(path, mtime_ms)` pairs. Agent sessions (files starting with `agent-`) are skipped.
pub fn walk_jsonl_files(root: &Path) -> Vec<(PathBuf, u64)> {
    walk_jsonl_paths(root)
        .into_iter()
        .filter(|path| !is_agent_session(path))
        .filter_map(|path| {
            let meta = fs::metadata(&path).ok()?;
            let mtime = meta.modified().ok()?;
            let mtime_ms = mtime
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            Some((path, mtime_ms))
        })
        .collect()
}

/// Walk a directory recursively, collecting all `.jsonl` file paths.
pub fn walk_jsonl_paths(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_jsonl_files(root, &mut files);
    files
}

fn collect_jsonl_files(root: &Path, files: &mut Vec<PathBuf>) {
    if !root.exists() {
        return;
    }

    let entries = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if should_descend_into_dir(&entry) {
            collect_jsonl_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

fn is_agent_session(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .map(|n| n.starts_with("agent-"))
        .unwrap_or(false)
}

fn should_descend_into_dir(entry: &fs::DirEntry) -> bool {
    let Ok(file_type) = entry.file_type() else {
        return false;
    };

    if !file_type.is_dir() || file_type.is_symlink() {
        return false;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            return false;
        };
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[cfg(unix)]
    fn create_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
        let link = link.to_string_lossy().replace('\'', "''");
        let target = target.to_string_lossy().replace('\'', "''");
        let command =
            format!("New-Item -ItemType Junction -Path '{link}' -Target '{target}' | Out-Null");
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &command])
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "mklink /J failed",
            ))
        }
    }

    #[test]
    fn walk_jsonl_files_skips_directory_link_cycle() {
        let temp = tempdir().expect("tempdir");
        let real = temp.path().join("real");
        std::fs::create_dir_all(&real).expect("real dir");
        std::fs::write(real.join("session.jsonl"), "{}\n").expect("session");

        let link = real.join("loop");
        create_dir_link(&real, &link).expect("create directory link");

        let files = walk_jsonl_files(temp.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, real.join("session.jsonl"));
    }
}
