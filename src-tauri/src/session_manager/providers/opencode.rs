use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::session_manager::{SessionMessage, SessionMeta, ToolCallInfo};

use super::utils::{move_single_file, path_basename, truncate_summary, TITLE_MAX_CHARS};
use super::SessionProvider;

const PROVIDER_ID: &str = "opencode";

// ─── OpenCodeProvider ───────────────────────────────────────────────────────

/// Provider implementation for OpenCode sessions.
///
/// Storage layout (JSON-only, no SQLite):
///   {base}/storage/
///     session/{project_id}/{session_id}.json   — session metadata
///     message/{session_id}/{message_id}.json    — messages
///     part/{message_id}/{part_id}.json          — message parts
pub struct OpenCodeProvider;

impl SessionProvider for OpenCodeProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![
            crate::config::get_opencode_base_dir().join("storage"),
            crate::config::get_opencode_archive_dir(),
        ]
    }

    fn scan_sessions(&self, root: &Path) -> Vec<SessionMeta> {
        scan_sessions_in_root(root)
    }

    fn load_messages(&self, path: &Path) -> Result<Vec<SessionMessage>, String> {
        let storage_dir = derive_storage_base(path);
        load_messages_internal(path, &storage_dir)
    }

    fn load_raw_content_fallback(&self, _path: &Path) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn parse_session(&self, path: &Path) -> Option<SessionMeta> {
        // Only handle .json files
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            return None;
        }
        let storage_dir = derive_storage_base(path);
        parse_session_file(path, &storage_dir)
    }

    fn move_session(&self, source: &Path, dest: &Path) -> Result<(), String> {
        move_session(source, dest)
    }

    fn user_events(&self, path: &Path) -> Result<Vec<String>, String> {
        let storage_dir = derive_storage_base(path);
        user_events_internal(path, &storage_dir)
    }
}

// ─── Path helpers ────────────────────────────────────────────────────────────

/// Derive the storage base directory from a session file path.
///
/// A session file lives at `{storage_dir}/session/{project_id}/{session_id}.json`,
/// so the storage dir is 3 levels up from the file.
fn derive_storage_base(session_path: &Path) -> PathBuf {
    session_path
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| crate::config::get_opencode_base_dir().join("storage"))
}

// ─── Internal functions ─────────────────────────────────────────────────────

fn scan_sessions_in_root(root: &Path) -> Vec<SessionMeta> {
    let session_root = root.join("session");
    if !session_root.is_dir() {
        return Vec::new();
    }

    let mut sessions = Vec::new();

    let project_dirs = match std::fs::read_dir(&session_root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    for entry in project_dirs.flatten() {
        let project_path = entry.path();
        if !project_path.is_dir() {
            continue;
        }

        let session_files = match std::fs::read_dir(&project_path) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for file_entry in session_files.flatten() {
            let path = file_entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Some(meta) = parse_session_file(&path, root) {
                sessions.push(meta);
            }
        }
    }

    sessions
}

/// Parse an OpenCode session JSON file and extract metadata.
fn parse_session_file(path: &Path, storage_dir: &Path) -> Option<SessionMeta> {
    let content = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&content).ok()?;

    let session_id = value.get("id")?.as_str()?.to_string();

    let title = value
        .get("title")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());

    let directory = value
        .get("directory")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let created_at = value
        .get("time")
        .and_then(|t| t.get("created"))
        .and_then(|v| v.as_i64());

    let last_active_at = value
        .get("time")
        .and_then(|t| t.get("updated"))
        .and_then(|v| v.as_i64())
        .or(created_at);

    // Title priority: session JSON title > directory basename > first user message summary
    let title = title
        .or_else(|| directory.as_deref().and_then(path_basename))
        .or_else(|| first_user_message_summary(storage_dir, &session_id));

    // source_path points to the session JSON file itself, consistent with other providers
    // (Claude/Codex/Gemini all set source_path to the session file path)
    let source_path = path.to_string_lossy().to_string();

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title: title.map(|t| truncate_summary(&t, TITLE_MAX_CHARS)),
        summary: None,
        project_dir: directory,
        created_at,
        last_active_at,
        source_path: Some(source_path),
        resume_command: Some(format!("opencode -s {session_id}")),
        forked_from_id: None,
    })
}

/// Read the first user message's text content for use as a title fallback.
fn first_user_message_summary(storage_dir: &Path, session_id: &str) -> Option<String> {
    let msg_dir = storage_dir.join("message").join(session_id);
    if !msg_dir.is_dir() {
        return None;
    }

    let mut messages: Vec<(i64, String)> = Vec::new();

    let entries = match std::fs::read_dir(&msg_dir) {
        Ok(entries) => entries,
        Err(_) => return None,
    };

    for entry in entries.flatten() {
        let msg_path = entry.path();
        if msg_path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let content = match std::fs::read_to_string(&msg_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if value.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }

        let created = value
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let msg_id = match value.get("id").and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => continue,
        };

        let part_dir = storage_dir.join("part").join(&msg_id);
        let text = read_part_text(&part_dir);
        if !text.is_empty() {
            messages.push((created, text));
        }
    }

    // Sort by time and take the first user message
    messages.sort_by_key(|(ts, _)| *ts);
    messages.into_iter().next().map(|(_, text)| text)
}

/// Read all text parts from a part directory and join them.
/// Entries are sorted by filename to ensure deterministic ordering across
/// platforms (Linux ext4 does not guarantee alphabetical readdir order).
fn read_part_text(part_dir: &Path) -> String {
    if !part_dir.is_dir() {
        return String::new();
    }

    let mut parts = Vec::new();
    let entries = match std::fs::read_dir(part_dir) {
        Ok(entries) => entries,
        Err(_) => return String::new(),
    };

    let mut paths: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    paths.sort();

    for part_path in paths {

        let content = match std::fs::read_to_string(&part_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match value.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = value.get("text").and_then(Value::as_str) {
                    parts.push(text.to_string());
                }
            }
            Some("tool") => {
                if let Some(tool) = value.get("tool").and_then(Value::as_str) {
                    parts.push(format!("[Tool: {tool}]"));
                }
            }
            _ => {}
        }
    }

    parts.join("\n")
}

/// Read tool calls from the parts directory.
/// OpenCode tool parts only have a name, no input payload.
fn read_part_tool_calls(part_dir: &Path) -> Vec<ToolCallInfo> {
    if !part_dir.is_dir() {
        return Vec::new();
    }

    let mut calls = Vec::new();
    let entries = match std::fs::read_dir(part_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let part_path = entry.path();
        if part_path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let content = match std::fs::read_to_string(&part_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if value.get("type").and_then(Value::as_str) == Some("tool") {
            if let Some(name) = value.get("tool").and_then(Value::as_str) {
                let input = value
                    .get("state")
                    .and_then(|s| s.get("input"))
                    .map(|i| i.to_string())
                    .unwrap_or_default();
                let call_id = value
                    .get("callID")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                calls.push(ToolCallInfo {
                    name: name.to_string(),
                    input,
                    call_id,
                });
            }
        }
    }

    calls
}

/// Load messages for a session given the session JSON file path and storage dir.
fn load_messages_internal(
    session_path: &Path,
    storage_dir: &Path,
) -> Result<Vec<SessionMessage>, String> {
    // Parse session JSON to get the session id
    let session_content = std::fs::read_to_string(session_path)
        .map_err(|e| format!("Failed to read session file: {e}"))?;
    let session_value: Value = serde_json::from_str(&session_content)
        .map_err(|e| format!("Failed to parse session JSON: {e}"))?;
    let session_id = session_value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing session id in file".to_string())?;

    let msg_dir = storage_dir.join("message").join(session_id);
    if !msg_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut messages_raw: Vec<(i64, String, String, Vec<ToolCallInfo>)> = Vec::new();

    let entries = std::fs::read_dir(&msg_dir)
        .map_err(|e| format!("Failed to read message directory: {e}"))?;

    for entry in entries.flatten() {
        let msg_path = entry.path();
        if msg_path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let msg_content = match std::fs::read_to_string(&msg_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let msg_value: Value = match serde_json::from_str(&msg_content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let role = msg_value
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        let created = msg_value
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let msg_id = match msg_value.get("id").and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => continue,
        };

        // Read parts for this message
        let part_dir = storage_dir.join("part").join(&msg_id);
        let text = read_part_text(&part_dir);
        if text.trim().is_empty() {
            continue;
        }
        let tool_calls = read_part_tool_calls(&part_dir);

        messages_raw.push((created, role, text, tool_calls));
    }

    // Sort by created timestamp
    messages_raw.sort_by_key(|(ts, _, _, _)| *ts);

    Ok(messages_raw
        .into_iter()
        .map(|(ts, role, content, tool_calls)| {
            let tc = if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            };
            SessionMessage {
                role,
                content,
                ts: Some(ts),
                usage: None,
                cumulative_usage: None,
                tool_calls: tc,
                tool_result: None,
            }
        })
        .collect())
}

/// Extract user input text events for fork tree hash chain computation.
/// Reads all user-role messages in chronological order and returns their text.
fn user_events_internal(session_path: &Path, storage_dir: &Path) -> Result<Vec<String>, String> {
    let session_content = std::fs::read_to_string(session_path)
        .map_err(|e| format!("Failed to read session file: {e}"))?;
    let session_value: Value = serde_json::from_str(&session_content)
        .map_err(|e| format!("Failed to parse session JSON: {e}"))?;
    let session_id = session_value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "Missing session id in file".to_string())?;

    let msg_dir = storage_dir.join("message").join(session_id);
    if !msg_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut events: Vec<(i64, String)> = Vec::new();

    let entries = std::fs::read_dir(&msg_dir)
        .map_err(|e| format!("Failed to read message directory: {e}"))?;

    for entry in entries.flatten() {
        let msg_path = entry.path();
        if msg_path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let msg_content = match std::fs::read_to_string(&msg_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let msg_value: Value = match serde_json::from_str(&msg_content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Only user messages
        if msg_value.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }

        let created = msg_value
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let msg_id = match msg_value.get("id").and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => continue,
        };

        let part_dir = storage_dir.join("part").join(&msg_id);
        let text = read_part_text(&part_dir);
        if text.trim().is_empty() {
            continue;
        }

        events.push((created, text));
    }

    // Sort by timestamp
    events.sort_by_key(|(ts, _)| *ts);

    Ok(events.into_iter().map(|(_, text)| text).collect())
}

fn move_session(source_path: &Path, dest_dir: &Path) -> Result<(), String> {
    use std::fs;
    // Move the session JSON file itself
    move_single_file(source_path, dest_dir)?;

    // Also move associated message and part directories
    let storage_base = derive_storage_base(source_path);
    let session_id = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if !session_id.is_empty() {
        let archive_base = dest_dir.parent().and_then(Path::parent);
        if let Some(archive) = archive_base {
            // Move message directory
            let msg_dir = storage_base.join("message").join(session_id);
            if msg_dir.exists() {
                let dest_msg = archive.join("message").join(session_id);
                if dest_msg.exists() {
                    fs::remove_dir_all(&dest_msg).ok();
                }
                fs::create_dir_all(dest_msg.parent().unwrap())
                    .map_err(|e| format!("Failed to create message parent dir: {e}"))?;
                fs::rename(&msg_dir, &dest_msg)
                    .map_err(|e| format!("Failed to move message dir: {e}"))?;

                // Move part directories for each message
                if let Ok(entries) = fs::read_dir(&dest_msg) {
                    for entry in entries.flatten() {
                        if let Some(msg_id) = entry.path().file_stem().and_then(|s| s.to_str()) {
                            let part_dir = storage_base.join("part").join(msg_id);
                            if part_dir.exists() {
                                let dest_part = archive.join("part").join(msg_id);
                                if dest_part.exists() {
                                    fs::remove_dir_all(&dest_part).ok();
                                }
                                if let Some(parent) = dest_part.parent() {
                                    fs::create_dir_all(parent).ok();
                                }
                                fs::rename(&part_dir, &dest_part).ok();
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Write a session JSON file into the given storage tree and return its path.
    fn write_session(
        storage: &Path,
        project_id: &str,
        session_id: &str,
        title: Option<&str>,
        directory: Option<&str>,
        created: i64,
        updated: Option<i64>,
    ) -> PathBuf {
        let session_dir = storage.join("session").join(project_id);
        std::fs::create_dir_all(&session_dir).expect("create session dir");

        let mut json = serde_json::Map::new();
        json.insert("id".into(), Value::String(session_id.to_string()));
        if let Some(t) = title {
            json.insert("title".into(), Value::String(t.to_string()));
        }
        if let Some(d) = directory {
            json.insert("directory".into(), Value::String(d.to_string()));
        }
        let mut time = serde_json::Map::new();
        time.insert("created".into(), Value::Number(created.into()));
        if let Some(u) = updated {
            time.insert("updated".into(), Value::Number(u.into()));
        }
        json.insert("time".into(), Value::Object(time));

        let path = session_dir.join(format!("{session_id}.json"));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json).expect("serialize"),
        )
        .expect("write session file");
        path
    }

    /// Write a message JSON file.
    fn write_message(storage: &Path, session_id: &str, msg_id: &str, role: &str, created: i64) {
        let msg_dir = storage.join("message").join(session_id);
        std::fs::create_dir_all(&msg_dir).expect("create message dir");

        let mut json = serde_json::Map::new();
        json.insert("id".into(), Value::String(msg_id.to_string()));
        json.insert("role".into(), Value::String(role.to_string()));
        json.insert("sessionID".into(), Value::String(session_id.to_string()));
        let mut time = serde_json::Map::new();
        time.insert("created".into(), Value::Number(created.into()));
        json.insert("time".into(), Value::Object(time));

        let path = msg_dir.join(format!("{msg_id}.json"));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json).expect("serialize"),
        )
        .expect("write message file");
    }

    /// Write a text part JSON file.
    fn write_text_part(storage: &Path, msg_id: &str, part_id: &str, text: &str) {
        let part_dir = storage.join("part").join(msg_id);
        std::fs::create_dir_all(&part_dir).expect("create part dir");

        let json = serde_json::json!({
            "id": part_id,
            "type": "text",
            "text": text,
        });

        let path = part_dir.join(format!("{part_id}.json"));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json).expect("serialize"),
        )
        .expect("write part file");
    }

    /// Write a tool part JSON file (simple form).
    fn write_tool_part(storage: &Path, msg_id: &str, part_id: &str, tool: &str) {
        let part_dir = storage.join("part").join(msg_id);
        std::fs::create_dir_all(&part_dir).expect("create part dir");

        let json = serde_json::json!({
            "id": part_id,
            "type": "tool",
            "tool": tool,
        });

        let path = part_dir.join(format!("{part_id}.json"));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json).expect("serialize"),
        )
        .expect("write part file");
    }

    /// Write a tool part with real-world fields: callID + state.input.
    fn write_tool_part_with_call(
        storage: &Path,
        msg_id: &str,
        part_id: &str,
        tool: &str,
        call_id: &str,
        command: &str,
    ) {
        let part_dir = storage.join("part").join(msg_id);
        std::fs::create_dir_all(&part_dir).expect("create part dir");

        let json = serde_json::json!({
            "id": part_id,
            "type": "tool",
            "callID": call_id,
            "tool": tool,
            "state": {
                "input": {
                    "command": command,
                    "description": "test command",
                },
                "output": "",
                "status": "completed",
            },
        });

        let path = part_dir.join(format!("{part_id}.json"));
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&json).expect("serialize"),
        )
        .expect("write part file");
    }

    /// Keep a tempdir alive for the test duration while providing the storage path.
    struct TestStorage {
        #[allow(dead_code)]
        dir: tempfile::TempDir,
        storage: PathBuf,
    }

    impl TestStorage {
        fn new() -> Self {
            let dir = tempdir().expect("tempdir");
            let storage = dir.path().join("storage");
            std::fs::create_dir_all(&storage).expect("create storage dir");
            TestStorage { dir, storage }
        }
    }

    // ─── Provider trait tests ─────────────────────────────────────────────────

    #[test]
    fn opencode_provider_trait_impl() {
        let provider = OpenCodeProvider;
        assert_eq!(provider.id(), "opencode");
        assert_eq!(provider.roots().len(), 2);
    }

    #[test]
    fn load_raw_content_fallback_returns_none() {
        let provider = OpenCodeProvider;
        let result = provider
            .load_raw_content_fallback(Path::new("/tmp/fake.json"))
            .expect("should succeed");
        assert!(result.is_none());
    }

    #[test]
    fn move_session_moves_file() {
        let temp = tempdir().expect("tempdir");
        let source_file = temp.path().join("session-test.json");
        std::fs::write(
            &source_file,
            r#"{"id":"move-test"}"#,
        )
        .expect("write");
        let dest_dir = temp.path().join("archived");
        let provider = OpenCodeProvider;
        provider
            .move_session(&source_file, &dest_dir)
            .expect("move should succeed");
        assert!(!source_file.exists(), "source file should be gone");
        assert!(
            dest_dir.join("session-test.json").exists(),
            "file should be at destination"
        );
    }

    // ─── parse_session tests ──────────────────────────────────────────────────

    #[test]
    fn parse_session_extracts_metadata() {
        let ts = TestStorage::new();
        let path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_123",
            Some("My Session Title"),
            Some("/home/user/my-project"),
            1_740_000_000_000,
            Some(1_740_003_600_000),
        );

        let provider = OpenCodeProvider;
        let meta = provider.parse_session(&path).expect("parse session");

        assert_eq!(meta.provider_id, "opencode");
        assert_eq!(meta.session_id, "ses_123");
        assert_eq!(meta.title.as_deref(), Some("My Session Title"));
        assert_eq!(meta.project_dir.as_deref(), Some("/home/user/my-project"));
        assert_eq!(meta.created_at, Some(1_740_000_000_000));
        assert_eq!(meta.last_active_at, Some(1_740_003_600_000));
        assert_eq!(meta.resume_command.as_deref(), Some("opencode -s ses_123"));
    }

    #[test]
    fn parse_session_uses_directory_basename_as_fallback() {
        let ts = TestStorage::new();
        let path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_456",
            None,
            Some("/home/user/my-project"),
            1_740_000_000_000,
            None,
        );

        let provider = OpenCodeProvider;
        let meta = provider.parse_session(&path).expect("parse session");

        assert_eq!(meta.session_id, "ses_456");
        assert_eq!(meta.title.as_deref(), Some("my-project"));
    }

    #[test]
    fn parse_session_uses_first_user_message_when_no_title_or_directory() {
        let ts = TestStorage::new();
        let path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_789",
            None, // no title
            None, // no directory
            1_740_000_000_000,
            None,
        );

        // Create a user message with parts
        write_message(&ts.storage, "ses_789", "msg_1", "user", 1_740_000_001_000);
        write_text_part(&ts.storage, "msg_1", "prt_1", "Hello world first message");

        let provider = OpenCodeProvider;
        let meta = provider.parse_session(&path).expect("parse session");

        assert_eq!(meta.session_id, "ses_789");
        assert_eq!(meta.title.as_deref(), Some("Hello world first message"));
    }

    // ─── validate_session_id tests ────────────────────────────────────────────

    #[test]
    fn validate_session_id_ok() {
        let ts = TestStorage::new();
        let path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_123",
            Some("Title"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        let provider = OpenCodeProvider;
        assert!(provider.validate_session_id(&path, "ses_123").is_ok());
    }

    #[test]
    fn validate_session_id_mismatch() {
        let ts = TestStorage::new();
        let path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_123",
            Some("Title"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        let provider = OpenCodeProvider;
        assert!(provider.validate_session_id(&path, "wrong-id").is_err());
    }

    // ─── load_messages tests ──────────────────────────────────────────────────

    #[test]
    fn load_messages_reads_messages_and_parts() {
        let ts = TestStorage::new();

        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_111",
            Some("Test Session"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        // Create user message with text parts
        write_message(&ts.storage, "ses_111", "msg_1", "user", 1_740_000_001_000);
        write_text_part(&ts.storage, "msg_1", "prt_1", "Hello");
        write_text_part(&ts.storage, "msg_1", "prt_2", "world");

        // Create assistant message with text part
        write_message(
            &ts.storage,
            "ses_111",
            "msg_2",
            "assistant",
            1_740_000_002_000,
        );
        write_text_part(&ts.storage, "msg_2", "prt_3", "How can I help?");

        let provider = OpenCodeProvider;
        let messages = provider
            .load_messages(&session_path)
            .expect("load messages");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello\nworld");
        assert_eq!(messages[0].ts, Some(1_740_000_001_000));

        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "How can I help?");
        assert_eq!(messages[1].ts, Some(1_740_000_002_000));
    }

    #[test]
    fn load_messages_includes_tool_parts() {
        let ts = TestStorage::new();

        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_222",
            Some("Tool Session"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        // User message
        write_message(&ts.storage, "ses_222", "msg_1", "user", 1_740_000_001_000);
        write_text_part(&ts.storage, "msg_1", "prt_1", "Run a command");

        // Assistant message with tool usage
        write_message(
            &ts.storage,
            "ses_222",
            "msg_2",
            "assistant",
            1_740_000_002_000,
        );
        write_text_part(&ts.storage, "msg_2", "prt_2", "Let me check");
        write_tool_part(&ts.storage, "msg_2", "prt_3", "bash");

        let provider = OpenCodeProvider;
        let messages = provider
            .load_messages(&session_path)
            .expect("load messages");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].role, "assistant");
        assert!(messages[1].content.contains("Let me check"));
        assert!(messages[1].content.contains("[Tool: bash]"));
    }

    #[test]
    fn load_messages_skips_messages_with_no_parts() {
        let ts = TestStorage::new();

        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_333",
            Some("Empty Msg"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        // Message with no parts dir — should be skipped
        write_message(&ts.storage, "ses_333", "msg_1", "user", 1_740_000_001_000);

        // Message with empty part — should be skipped
        write_message(&ts.storage, "ses_333", "msg_2", "user", 1_740_000_002_000);
        let empty_part_dir = ts.storage.join("part").join("msg_2");
        std::fs::create_dir_all(&empty_part_dir).expect("create part dir");
        let empty_part = serde_json::json!({"id": "prt_empty", "type": "text", "text": ""});
        std::fs::write(
            empty_part_dir.join("prt_empty.json"),
            serde_json::to_string_pretty(&empty_part).unwrap(),
        )
        .expect("write empty part");

        // Valid message
        write_message(
            &ts.storage,
            "ses_333",
            "msg_3",
            "assistant",
            1_740_000_003_000,
        );
        write_text_part(&ts.storage, "msg_3", "prt_1", "Valid response");

        let messages = load_messages_internal(&session_path, &ts.storage).expect("load messages");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Valid response");
    }

    // ─── source_path fixture tests ────────────────────────────────────────────

    #[test]
    fn parse_session_source_path_points_to_existing_file() {
        let ts = TestStorage::new();
        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_sp_1",
            Some("Source Path Test"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        // Parse via trait method (simulates real flow)
        let provider = OpenCodeProvider;
        let meta = provider
            .parse_session(&session_path)
            .expect("parse session");

        // source_path must point to an existing file (previously a directory path caused read_to_string to crash)
        let sp = meta.source_path.expect("source_path should be present");
        let sp_path = std::path::Path::new(&sp);
        assert!(sp_path.exists(), "source_path '{sp}' should exist");
        assert!(
            sp_path.is_file(),
            "source_path '{sp}' should be a file, not a directory"
        );
    }

    #[test]
    fn load_messages_works_with_source_path_from_parse_session() {
        let ts = TestStorage::new();
        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_sp_2",
            Some("Full Flow"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        // Create messages
        write_message(&ts.storage, "ses_sp_2", "msg_f1", "user", 1_740_000_001_000);
        write_text_part(&ts.storage, "msg_f1", "prt_f1", "Hello from flow");
        write_message(
            &ts.storage,
            "ses_sp_2",
            "msg_f2",
            "assistant",
            1_740_000_002_000,
        );
        write_text_part(&ts.storage, "msg_f2", "prt_f2", "Hi there");

        // Simulate the real frontend flow: parse → get source_path → load_messages
        let provider = OpenCodeProvider;
        let meta = provider
            .parse_session(&session_path)
            .expect("parse session");
        let sp = meta.source_path.expect("source_path");
        let messages = provider
            .load_messages(std::path::Path::new(&sp))
            .expect("load_messages via source_path");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "Hello from flow");
        assert_eq!(messages[1].content, "Hi there");
    }

    #[test]
    fn load_messages_tool_calls_extract_call_id_and_input() {
        let ts = TestStorage::new();
        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_tc_1",
            Some("Tool Call"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        write_message(&ts.storage, "ses_tc_1", "msg_t1", "user", 1_740_000_001_000);
        write_text_part(&ts.storage, "msg_t1", "prt_t1", "Run command");

        write_message(
            &ts.storage,
            "ses_tc_1",
            "msg_t2",
            "assistant",
            1_740_000_002_000,
        );
        write_text_part(&ts.storage, "msg_t2", "prt_t2", "Running...");
        // Tool part with real-world callID + state.input.command
        write_tool_part_with_call(
            &ts.storage,
            "msg_t2",
            "prt_t3",
            "bash",
            "call_79398764692c484892159dad",
            "python -m venv venv",
        );

        let provider = OpenCodeProvider;
        let meta = provider
            .parse_session(&session_path)
            .expect("parse session");
        let sp = meta.source_path.expect("source_path");
        let messages = provider
            .load_messages(std::path::Path::new(&sp))
            .expect("load_messages");

        assert_eq!(messages.len(), 2);
        let assistant = &messages[1];
        assert_eq!(assistant.role, "assistant");

        // tool_calls should be present with call_id and input
        let tool_calls = assistant
            .tool_calls
            .as_ref()
            .expect("should have tool_calls");
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "bash");
        assert_eq!(
            tool_calls[0].call_id.as_deref(),
            Some("call_79398764692c484892159dad"),
            "callID from tool part should be captured"
        );
        assert!(
            tool_calls[0].input.contains("python -m venv venv"),
            "state.input.command should be captured, got: {}",
            tool_calls[0].input,
        );
    }

    // ─── scan_sessions tests ──────────────────────────────────────────────────

    #[test]
    fn scan_sessions_finds_json_files() {
        let ts = TestStorage::new();
        let storage_dir = &ts.storage;

        write_session(
            storage_dir,
            "proj_abc",
            "ses_001",
            Some("First Session"),
            Some("/tmp/proj-a"),
            1_740_000_000_000,
            None,
        );
        write_session(
            storage_dir,
            "proj_xyz",
            "ses_002",
            Some("Second Session"),
            Some("/tmp/proj-b"),
            1_740_000_001_000,
            None,
        );

        // Non-JSON file should be ignored
        let other_dir = storage_dir.join("session").join("proj_abc");
        std::fs::write(other_dir.join("notes.txt"), "not a session").expect("write notes");

        let sessions = scan_sessions_in_root(storage_dir);
        assert_eq!(sessions.len(), 2);
        let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert!(ids.contains(&"ses_001"));
        assert!(ids.contains(&"ses_002"));
    }

    #[test]
    fn scan_sessions_empty_dir_returns_empty() {
        let ts = TestStorage::new();
        let sessions = scan_sessions_in_root(&ts.storage);
        assert!(sessions.is_empty());
    }

    #[test]
    fn scan_sessions_ignores_non_session_files_in_root() {
        let ts = TestStorage::new();
        std::fs::write(ts.storage.join("random.json"), "{}").expect("write random");

        let sessions = scan_sessions_in_root(&ts.storage);
        assert!(sessions.is_empty());
    }

    // ─── user_events tests ──────────────────────────────────────────────────────

    #[test]
    fn user_events_returns_user_texts_in_order() {
        let ts = TestStorage::new();

        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_ue_1",
            Some("User Events"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        // User messages with text parts
        write_message(
            &ts.storage,
            "ses_ue_1",
            "msg_ue1",
            "user",
            1_740_000_001_000,
        );
        write_text_part(&ts.storage, "msg_ue1", "p0", "First user input");

        write_message(
            &ts.storage,
            "ses_ue_1",
            "msg_ue2",
            "user",
            1_740_000_003_000,
        );
        write_text_part(&ts.storage, "msg_ue2", "p0", "Second user input");

        // Assistant message — should be ignored
        write_message(
            &ts.storage,
            "ses_ue_1",
            "msg_ue3",
            "assistant",
            1_740_000_002_000,
        );
        write_text_part(&ts.storage, "msg_ue3", "p0", "Assistant reply");

        let provider = OpenCodeProvider;
        let events = provider.user_events(&session_path).expect("user_events");
        assert_eq!(events.len(), 2, "only user messages");
        assert_eq!(events[0], "First user input");
        assert_eq!(events[1], "Second user input");
    }

    #[test]
    fn user_events_skips_tool_result_only_user_messages() {
        let ts = TestStorage::new();

        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_ue_2",
            Some("Tool Result Only"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        // User message that is a tool result — has text but no role=user content
        write_message(&ts.storage, "ses_ue_2", "msg_t1", "user", 1_740_000_001_000);
        write_text_part(&ts.storage, "msg_t1", "p0", "tool output");

        let events = provider_user_events_internal_wrapper(&session_path, &ts.storage);
        assert_eq!(
            events.len(),
            1,
            "tool-result user messages still count as user events"
        );
        assert_eq!(events[0], "tool output");
    }

    /// Helper to call user_events_internal directly for testing.
    fn provider_user_events_internal_wrapper(session_path: &Path, storage: &Path) -> Vec<String> {
        user_events_internal(session_path, storage).expect("user_events_internal")
    }
}
