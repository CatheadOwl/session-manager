use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{
    extract_text, extract_tool_calls, extract_tool_results, move_single_file,
    parse_timestamp_to_ms, read_head_tail_lines, truncate_summary, TITLE_MAX_CHARS,
};
use super::SessionProvider;

const PROVIDER_ID: &str = "hermes";

// ─── HermesProvider ──────────────────────────────────────────────────────────

/// Provider implementation for Hermes sessions (.jsonl files in ~/.config/hermes/sessions/).
pub struct HermesProvider;

impl SessionProvider for HermesProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![
            crate::config::get_hermes_sessions_dir(),
            crate::config::get_hermes_archive_dir(),
        ]
    }

    fn scan_sessions(&self, root: &Path) -> Vec<SessionMeta> {
        scan_sessions_in_root(root)
    }

    fn load_messages(&self, path: &Path) -> Result<Vec<SessionMessage>, String> {
        load_messages(path)
    }

    fn load_raw_content_fallback(&self, _path: &Path) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn parse_session(&self, path: &Path) -> Option<SessionMeta> {
        parse_session(path)
    }

    fn move_session(&self, source: &Path, dest: &Path) -> Result<(), String> {
        move_session(source, dest)
    }

}

// ─── Internals ────────────────────────────────────────────────────────────────

fn scan_sessions_in_root(root: &Path) -> Vec<SessionMeta> {
    if !root.exists() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("jsonl") && ext != Some("json") {
            continue;
        }
        if let Some(meta) = parse_session(&path) {
            sessions.push(meta);
        }
    }
    sessions
}

fn parse_session(path: &Path) -> Option<SessionMeta> {
    // Read head (metadata + first user message) and tail (last timestamp)
    let (head, tail) = read_head_tail_lines(path, 30, 10).ok()?;

    let mut first_user_msg: Option<String> = None;
    let mut first_ts: Option<i64> = None;
    let mut last_ts: Option<i64> = None;
    let mut session_id: Option<String> = None;
    let mut title: Option<String> = None;
    let mut cwd: Option<String> = None;
    let mut found_session_marker: bool = false;

    // Process head lines for metadata and first user message
    for line in &head {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let ts = value
            .get("timestamp")
            .or_else(|| value.get("ts"))
            .and_then(parse_timestamp_to_ms);

        if first_ts.is_none() {
            first_ts = ts;
        }
        last_ts = ts.or(last_ts);

        let line_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        // Extract session metadata from session-type lines
        if line_type == "session" || line_type == "init" {
            found_session_marker = true;
            if session_id.is_none() {
                session_id = value
                    .get("id")
                    .or_else(|| value.get("sessionId"))
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
            }
            if title.is_none() {
                title = value
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
            }
            if cwd.is_none() {
                cwd = value
                    .get("cwd")
                    .or_else(|| value.get("directory"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());
            }
        }

        if first_user_msg.is_none() {
            let role = value
                .get("role")
                .or_else(|| value.get("message").and_then(|m| m.get("role")))
                .and_then(Value::as_str);

            if role == Some("user") {
                let content = value
                    .get("content")
                    .or_else(|| value.get("message").and_then(|m| m.get("content")));
                if let Some(c) = content {
                    let text = extract_text(c);
                    if !text.trim().is_empty() {
                        first_user_msg = Some(truncate_summary(&text, TITLE_MAX_CHARS).to_string());
                    }
                }
            }
        }
    }

    // Process tail lines for the most recent timestamp
    for line in tail.iter().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let ts = value
            .get("timestamp")
            .or_else(|| value.get("ts"))
            .and_then(parse_timestamp_to_ms);
        if let Some(t) = ts {
            last_ts = Some(t);
            break;
        }
    }

    // Require actual Hermes-specific content (session/init line) to match.
    // Without it, a filename-only fallback would false-positive on non-Hermes .jsonl files.
    if !found_session_marker {
        return None;
    }

    // Fall back to filename as session ID
    let session_id = session_id.unwrap_or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown")
            .to_string()
    });

    // Title priority: JSON `title` field > first user message
    let resolved_title = title
        .map(|t| truncate_summary(&t, TITLE_MAX_CHARS))
        .or_else(|| first_user_msg.clone());

    let source_path = path.to_string_lossy().to_string();

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id,
        title: resolved_title,
        summary: first_user_msg,
        project_dir: cwd,
        created_at: first_ts,
        last_active_at: last_ts.or(first_ts),
        source_path: Some(source_path),
        resume_command: None,
        forked_from_id: None,
    })
}

/// Load messages from a Hermes JSONL file.
fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Support both flat messages and nested {type:"message", message:{...}} format
        let (role_val, content_val, ts_val) =
            if value.get("type").and_then(Value::as_str) == Some("message") {
                let msg = match value.get("message") {
                    Some(m) => m,
                    None => continue,
                };
                (
                    msg.get("role"),
                    msg.get("content"),
                    value.get("timestamp").or_else(|| msg.get("ts")),
                )
            } else {
                (
                    value.get("role"),
                    value.get("content"),
                    value.get("timestamp").or_else(|| value.get("ts")),
                )
            };

        let role = match role_val.and_then(Value::as_str) {
            Some(r) => r.to_string(),
            None => continue,
        };

        let content = content_val.map(extract_text).unwrap_or_default();
        if content.trim().is_empty() {
            continue;
        }
        let tool_calls = content_val.and_then(|v| {
            let calls = extract_tool_calls(v);
            if calls.is_empty() {
                None
            } else {
                Some(calls)
            }
        });
        let tool_result = content_val.and_then(extract_tool_results);

        let ts = ts_val.and_then(parse_timestamp_to_ms);
        messages.push(SessionMessage {
            role,
            content,
            ts,
            usage: None,
            cumulative_usage: None,
            tool_calls,
            tool_result,
        });
    }

    Ok(messages)
}

fn move_session(source_path: &Path, dest_dir: &Path) -> Result<(), String> {
    move_single_file(source_path, dest_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;

    #[test]
    fn hermes_provider_trait_impl() {
        let provider = HermesProvider;
        assert_eq!(provider.id(), "hermes");
        assert_eq!(provider.roots().len(), 2);
    }

    #[test]
    fn parse_session_extracts_metadata() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("test-session.jsonl");
        let mut f = File::create(&path).expect("create");
        writeln!(
            f,
            r#"{{"type":"session","id":"s1","title":"My Session","cwd":"/home/user/project"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"user","content":"Hello world"}},"timestamp":"2026-01-01T00:00:00Z"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","content":"Hi there"}},"timestamp":"2026-01-01T00:01:00Z"}}"#
        )
        .unwrap();
        f.flush().unwrap();

        let meta = parse_session(&path).expect("should parse");
        assert_eq!(meta.session_id, "s1");
        assert_eq!(meta.title.as_deref(), Some("My Session"));
        assert_eq!(meta.project_dir.as_deref(), Some("/home/user/project"));
        assert!(meta.created_at.is_some());
        assert!(meta.last_active_at.is_some());
        assert_eq!(meta.resume_command, None);
        assert_eq!(meta.provider_id, "hermes");
    }

    #[test]
    fn parse_session_fallback_to_first_user_message() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("test-session.jsonl");
        let mut f = File::create(&path).expect("create");
        writeln!(
            f,
            r#"{{"type":"session","id":"s1","cwd":"/home/user/project"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"role":"user","content":"Hello world","ts":1700000000}}"#
        )
        .unwrap();
        f.flush().unwrap();

        let meta = parse_session(&path).expect("should parse");
        assert_eq!(meta.session_id, "s1");
        assert_eq!(meta.title.as_deref(), Some("Hello world"));
    }

    #[test]
    fn parse_session_rejects_no_session_marker() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("my-session.jsonl");
        let mut f = File::create(&path).expect("create");
        // No type == "session" or "init" line — not a valid Hermes file
        writeln!(
            f,
            r#"{{"role":"assistant","content":"Hi","ts":1700000000}}"#
        )
        .unwrap();
        f.flush().unwrap();

        let meta = parse_session(&path);
        assert!(
            meta.is_none(),
            "should reject file without session/init marker"
        );
    }

    #[test]
    fn load_messages_flat_format() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");
        let mut f = File::create(&path).expect("create");
        writeln!(
            f,
            r#"{{"role":"user","content":"What is Rust?","ts":1700000000}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"role":"assistant","content":"A systems programming language.","ts":1700000001}}"#
        )
        .unwrap();
        f.flush().unwrap();

        let msgs = load_messages(&path).expect("should load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "What is Rust?");
        assert_eq!(msgs[0].ts, Some(1700000000000));
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "A systems programming language.");
    }

    #[test]
    fn load_messages_nested_format() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");
        let mut f = File::create(&path).expect("create");
        writeln!(f, r#"{{"type":"session","id":"s1"}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"user","content":"Hello"}},"timestamp":"2026-01-01T00:00:00Z"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"assistant","content":"Hi"}},"timestamp":"2026-01-01T00:01:00Z"}}"#
        )
        .unwrap();
        f.flush().unwrap();

        let msgs = load_messages(&path).expect("should load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "Hello");
        assert!(msgs[0].ts.is_some());
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "Hi");
    }

    #[test]
    fn validate_session_id_ok() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");
        let mut f = File::create(&path).expect("create");
        writeln!(f, r#"{{"type":"session","id":"s1"}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"user","content":"hi"}}}}"#
        )
        .unwrap();
        f.flush().unwrap();

        let provider = HermesProvider;
        assert!(provider.validate_session_id(&path, "s1").is_ok());
    }

    #[test]
    fn validate_session_id_mismatch() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");
        let mut f = File::create(&path).expect("create");
        writeln!(f, r#"{{"type":"session","id":"s1"}}"#).unwrap();
        writeln!(
            f,
            r#"{{"type":"message","message":{{"role":"user","content":"hi"}}}}"#
        )
        .unwrap();
        f.flush().unwrap();

        let provider = HermesProvider;
        assert!(provider.validate_session_id(&path, "wrong-id").is_err());
    }

    #[test]
    fn move_session_moves_file() {
        let temp = tempdir().expect("tempdir");
        let source_file = temp.path().join("session.jsonl");
        std::fs::write(
            &source_file,
            "{\"id\":\"test\",\"messages\":[]}\n",
        )
        .expect("write");
        let dest_dir = temp.path().join("archived");
        let provider = HermesProvider;
        provider
            .move_session(&source_file, &dest_dir)
            .expect("move should succeed");
        assert!(!source_file.exists(), "source file should be gone");
        assert!(
            dest_dir.join("session.jsonl").exists(),
            "file should be at destination"
        );
    }

    #[test]
    fn load_raw_content_fallback_returns_none() {
        let provider = HermesProvider;
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("session.jsonl");
        std::fs::write(&path, "{}").expect("write");
        assert!(provider.load_raw_content_fallback(&path).unwrap().is_none());
    }

    #[test]
    fn scan_sessions_finds_jsonl_files() {
        let dir = tempdir().expect("tempdir");

        // Create some session files
        let f1_path = dir.path().join("session-1.jsonl");
        let mut f1 = File::create(&f1_path).expect("create");
        writeln!(f1, r#"{{"type":"session","id":"s1"}}"#).unwrap();
        writeln!(f1, r#"{{"role":"user","content":"hello"}}"#).unwrap();
        f1.flush().unwrap();

        let f2_path = dir.path().join("session-2.jsonl");
        let mut f2 = File::create(&f2_path).expect("create");
        writeln!(f2, r#"{{"type":"session","id":"s2","title":"Second"}}"#).unwrap();
        writeln!(f2, r#"{{"role":"user","content":"world"}}"#).unwrap();
        f2.flush().unwrap();

        // Create a .json file (should also be picked up)
        let f3_path = dir.path().join("session-3.json");
        let mut f3 = File::create(&f3_path).expect("create");
        writeln!(f3, r#"{{"type":"session","id":"s3"}}"#).unwrap();
        writeln!(f3, r#"{{"role":"user","content":"test"}}"#).unwrap();
        f3.flush().unwrap();

        let provider = HermesProvider;
        let sessions = provider.scan_sessions(dir.path());

        assert_eq!(sessions.len(), 3);

        let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert!(ids.contains(&"s1"));
        assert!(ids.contains(&"s2"));
        assert!(ids.contains(&"s3"));

        // Second session has a title
        let s2 = sessions.iter().find(|s| s.session_id == "s2").unwrap();
        assert_eq!(s2.title.as_deref(), Some("Second"));
    }

    #[test]
    fn scan_sessions_skips_non_jsonl_files() {
        let dir = tempdir().expect("tempdir");

        let txt_path = dir.path().join("notes.txt");
        std::fs::write(&txt_path, "not a session").expect("write");

        let provider = HermesProvider;
        let sessions = provider.scan_sessions(dir.path());
        assert!(sessions.is_empty());
    }
}
