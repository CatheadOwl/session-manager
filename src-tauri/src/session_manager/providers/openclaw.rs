use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{
    extract_text, extract_tool_calls, extract_tool_results, infer_session_id_from_filename,
    move_single_file, parse_timestamp_to_ms, path_basename, read_head_tail_lines,
    truncate_summary, TITLE_MAX_CHARS,
};
use super::SessionProvider;

const PROVIDER_ID: &str = "openclaw";

// ─── OpenClawProvider ────────────────────────────────────────────────────────

/// Provider implementation for OpenClaw sessions (.jsonl files in ~/.openclaw/agents/).
pub struct OpenClawProvider;

impl SessionProvider for OpenClawProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![
            crate::config::get_openclaw_agents_dir(),
            crate::config::get_openclaw_archive_dir(),
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

// ─── Internal functions ──────────────────────────────────────────────────────

fn scan_sessions_in_root(root: &Path) -> Vec<SessionMeta> {
    if !root.exists() {
        return Vec::new();
    }

    let mut sessions = Vec::new();

    let agent_dirs = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    for entry in agent_dirs.flatten() {
        let agent_path = entry.path();
        if !agent_path.is_dir() {
            continue;
        }

        let sessions_dir = agent_path.join("sessions");
        if !sessions_dir.is_dir() {
            continue;
        }

        // Load display names from sessions.json index
        let display_names = load_display_names(&sessions_dir);

        // Iterate JSONL session files
        let dir_entries = match std::fs::read_dir(&sessions_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for file_entry in dir_entries.flatten() {
            let path = file_entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(meta) = parse_session_with_options(&path, &display_names) {
                sessions.push(meta);
            }
        }
    }

    sessions
}

/// Read the `sessions.json` index file and build a sessionId -> displayName map.
fn load_display_names(sessions_dir: &Path) -> HashMap<String, String> {
    let index_path = sessions_dir.join("sessions.json");
    let content = match std::fs::read_to_string(&index_path) {
        Ok(c) => c,
        Err(_) => return HashMap::new(),
    };
    let value: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return HashMap::new(),
    };

    let mut names = HashMap::new();
    if let Value::Object(map) = value {
        for (_key, val) in map {
            let session_id = val
                .get("sessionId")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            let display_name = val
                .get("displayName")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            if let (Some(sid), Some(dn)) = (session_id, display_name) {
                names.insert(sid, dn);
            }
        }
    }
    names
}

fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        // Only process message-type events
        if value.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }

        let msg_obj = match value.get("message") {
            Some(msg) => msg,
            None => continue,
        };

        let role = match msg_obj.get("role").and_then(Value::as_str) {
            Some("toolResult") => "tool",
            Some(r) => r,
            None => "unknown",
        };

        let content_val = msg_obj.get("content");
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

        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);

        messages.push(SessionMessage {
            role: role.to_string(),
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

/// Parse session metadata without display name context (called from the trait).
fn parse_session(path: &Path) -> Option<SessionMeta> {
    parse_session_with_options(path, &HashMap::new())
}

/// Parse session metadata with an optional displayName lookup map.
fn parse_session_with_options(
    path: &Path,
    display_names: &HashMap<String, String>,
) -> Option<SessionMeta> {
    let (head, tail) = read_head_tail_lines(path, 10, 30).ok()?;

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut first_user_message: Option<String> = None;
    let mut found_session: bool = false;

    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        let event_type = value.get("type").and_then(Value::as_str);

        if event_type == Some("session") {
            found_session = true;
            // Session metadata event
            if session_id.is_none() {
                session_id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
            }
            if project_dir.is_none() {
                project_dir = value
                    .get("cwd")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
            }
            if created_at.is_none() {
                created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
            }
        } else if event_type == Some("message") && first_user_message.is_none() {
            // Message event — capture the first user message for title fallback
            if let Some(msg) = value.get("message") {
                if msg.get("role").and_then(Value::as_str) == Some("user") {
                    let text = msg.get("content").map(extract_text).unwrap_or_default();
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        first_user_message = Some(trimmed.to_string());
                    }
                }
            }
        }

        if session_id.is_some()
            && project_dir.is_some()
            && created_at.is_some()
            && first_user_message.is_some()
        {
            break;
        }
    }

    // Tail: find the last timestamp for last_active_at
    let mut last_active_at: Option<i64> = None;
    for line in tail.iter().rev() {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if last_active_at.is_none() {
            last_active_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if last_active_at.is_some() {
            break;
        }
    }

    // Require actual OpenClaw-specific content (type == "session" line) to match.
    if !found_session {
        return None;
    }

    let session_id = session_id.or_else(|| infer_session_id_from_filename(path));
    let session_id = session_id?;

    // Title priority: displayName > first user message > cwd basename
    let title = display_names
        .get(&session_id)
        .map(|n| truncate_summary(n, TITLE_MAX_CHARS))
        .or_else(|| first_user_message.map(|t| truncate_summary(&t, TITLE_MAX_CHARS)))
        .or_else(|| project_dir.as_deref().and_then(path_basename));

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id,
        title,
        summary: None,
        project_dir,
        created_at,
        last_active_at,
        source_path: Some(path.to_string_lossy().to_string()),
        resume_command: None,
        forked_from_id: None,
    })
}

fn move_session(source_path: &Path, dest_dir: &Path) -> Result<(), String> {
    move_single_file(source_path, dest_dir)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_session_uses_first_user_message_as_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"session\",\"id\":\"session-abc\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"How do I deploy?\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("How do I deploy?"));
        assert_eq!(meta.provider_id, "openclaw");
        assert_eq!(meta.resume_command, None);
    }

    #[test]
    fn parse_session_display_name_overrides_user_message() {
        let temp = tempdir().expect("tempdir");
        let sessions_dir = temp.path().join("sessions");
        std::fs::create_dir_all(&sessions_dir).expect("create sessions dir");

        // Write sessions.json index with displayName
        std::fs::write(
            sessions_dir.join("sessions.json"),
            r#"{"agent:main:main":{"sessionId":"session-abc","displayName":"重构登录模块"}}"#,
        )
        .expect("write sessions.json");

        // Write session file with a user message
        let path = sessions_dir.join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"session\",\"id\":\"session-abc\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"How do I deploy?\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
        .expect("write session");

        let display_names = load_display_names(&sessions_dir);
        let meta = parse_session_with_options(&path, &display_names).unwrap();
        assert_eq!(meta.title.as_deref(), Some("重构登录模块"));
    }

    #[test]
    fn parse_session_falls_back_to_dir_basename() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"session\",\"id\":\"session-abc\",\"cwd\":\"/tmp/my-project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("my-project"));
    }

    #[test]
    fn load_messages_parses_roles() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":\"world\"},\"timestamp\":\"2026-03-06T10:00:01Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"toolResult\",\"content\":\"output\"},\"timestamp\":\"2026-03-06T10:00:02Z\"}\n",
                "{\"type\":\"session\",\"id\":\"s1\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
            ),
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "world");
        assert_eq!(msgs[2].role, "tool");
        assert_eq!(msgs[2].content, "output");
    }

    #[test]
    fn load_messages_skips_empty_content() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"\"},\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"content\":\"  \"},\"timestamp\":\"2026-03-06T10:00:01Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"valid\"},\"timestamp\":\"2026-03-06T10:00:02Z\"}\n",
            ),
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "valid");
    }

    #[test]
    fn validate_session_id_ok() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"session\",\"id\":\"session-abc\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"hi\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
        .expect("write");

        let provider = OpenClawProvider;
        assert!(provider.validate_session_id(&path, "session-abc").is_ok());
    }

    #[test]
    fn validate_session_id_mismatch() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"session\",\"id\":\"session-abc\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
            ),
        )
        .expect("write");

        let provider = OpenClawProvider;
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
        let provider = OpenClawProvider;
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
    fn scan_sessions_finds_session_files_in_agent_dirs() {
        let temp = tempdir().expect("tempdir");
        let agent_dir = temp.path().join("my-agent").join("sessions");
        std::fs::create_dir_all(&agent_dir).expect("create agent sessions dir");

        // Write a session file
        std::fs::write(
            agent_dir.join("session-abc.jsonl"),
            concat!(
                "{\"type\":\"session\",\"id\":\"session-abc\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
        .expect("write session");

        // Write sessions.json index with display name
        std::fs::write(
            agent_dir.join("sessions.json"),
            r#"{"agent:main:main":{"sessionId":"session-abc","displayName":"My Agent Session"}}"#,
        )
        .expect("write sessions.json");

        // Write a non-JSONL file that should be ignored
        std::fs::write(agent_dir.join("notes.txt"), "not a session").expect("write notes");

        let sessions = scan_sessions_in_root(temp.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "session-abc");
        assert_eq!(sessions[0].title.as_deref(), Some("My Agent Session"));
    }

    #[test]
    fn scan_sessions_empty_dir_returns_empty() {
        let temp = tempdir().expect("tempdir");
        let sessions = scan_sessions_in_root(temp.path());
        assert!(sessions.is_empty());
    }

    #[test]
    fn openclaw_provider_trait_impl() {
        let provider = OpenClawProvider;
        assert_eq!(provider.id(), "openclaw");
        assert_eq!(provider.roots().len(), 2);
    }

    #[test]
    fn load_raw_content_fallback_returns_none() {
        let provider = OpenClawProvider;
        let result = provider
            .load_raw_content_fallback(Path::new("/tmp/fake.jsonl"))
            .expect("should succeed");
        assert!(result.is_none());
    }
}
