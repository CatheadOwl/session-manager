use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::session_manager::{SessionMessage, SessionMeta, ToolCallInfo};

use super::utils::{move_single_file, parse_timestamp_to_ms, truncate_summary, truncate_tool_input};
use super::SessionProvider;

const PROVIDER_ID: &str = "gemini";
const TITLE_MAX_CHARS: usize = 80;

// ─── GeminiProvider ─────────────────────────────────────────────────────────

/// Provider implementation for Google Gemini CLI sessions (.json files in ~/.gemini/tmp/).
pub struct GeminiProvider;

impl SessionProvider for GeminiProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![
            crate::config::get_gemini_dir().join("tmp"),
            crate::config::get_gemini_archive_dir(),
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

// ─── Internal functions ─────────────────────────────────────────────────────

fn scan_sessions_in_root(root: &Path) -> Vec<SessionMeta> {
    if !root.exists() {
        return Vec::new();
    }

    let mut sessions = Vec::new();

    // Iterate over project directories: tmp/<project_name>/chats/session-*.json
    let project_dirs = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    for entry in project_dirs.flatten() {
        let project_path = entry.path();
        if !project_path.is_dir() {
            continue;
        }

        let chats_dir = project_path.join("chats");
        if !chats_dir.is_dir() {
            continue;
        }

        // Read .project_root for project_dir metadata
        let project_root_file = project_path.join(".project_root");
        let project_dir = std::fs::read_to_string(&project_root_file)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let chat_files = match std::fs::read_dir(&chats_dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };

        for file_entry in chat_files.flatten() {
            let path = file_entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Some(mut meta) = parse_session(&path) {
                if meta.project_dir.is_none() {
                    meta.project_dir = project_dir.clone();
                }
                sessions.push(meta);
            }
        }
    }

    sessions
}

fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let data = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read Gemini session file: {e}"))?;
    let value: Value = serde_json::from_str(&data)
        .map_err(|e| format!("Failed to parse Gemini session JSON: {e}"))?;

    let messages = value
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| "No messages array found in Gemini session".to_string())?;

    let mut result = Vec::new();
    for msg in messages {
        let role = match msg.get("type").and_then(Value::as_str) {
            Some("gemini") => "assistant",
            Some("user") => "user",
            Some("info") | Some("error") => continue,
            _ => continue,
        };

        // Gemini content may be a plain string or an array of {text: ...} objects
        let mut content = match msg.get("content") {
            Some(Value::String(s)) => s.to_string(),
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => String::new(),
        };

        // Extract tool call info from the optional toolCalls array
        let mut tool_calls: Option<Vec<ToolCallInfo>> = None;
        if let Some(Value::Array(calls)) = msg.get("toolCalls") {
            let mut extracted: Vec<ToolCallInfo> = Vec::new();
            for call in calls {
                if let Some(name) = call.get("name").and_then(Value::as_str) {
                    // Gemini uses `args` or `arguments` for the tool input
                    let input = call
                        .get("args")
                        .or_else(|| call.get("arguments"))
                        .map(truncate_tool_input)
                        .unwrap_or_default();
                    if !content.is_empty() && !content.ends_with('\n') {
                        content.push('\n');
                    }
                    content.push_str(&format!("[Tool: {name}]"));
                    extracted.push(ToolCallInfo {
                        name: name.to_string(),
                        input,
                        call_id: None,
                    });
                }
            }
            if !extracted.is_empty() {
                tool_calls = Some(extracted);
            }
        }

        if content.trim().is_empty() {
            continue;
        }

        let ts = msg.get("timestamp").and_then(parse_timestamp_to_ms);

        result.push(SessionMessage {
            role: role.to_string(),
            content,
            ts,
            usage: None,
            cumulative_usage: None,
            tool_calls,
            tool_result: None,
        });
    }

    Ok(result)
}

fn parse_session(path: &Path) -> Option<SessionMeta> {
    let data = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&data).ok()?;

    let session_id = value.get("sessionId").and_then(Value::as_str)?.to_string();

    let created_at = value.get("startTime").and_then(parse_timestamp_to_ms);
    let last_active_at = value.get("lastUpdated").and_then(parse_timestamp_to_ms);

    // Derive title from first user message
    let title = value
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|msgs| {
            msgs.iter()
                .find(|m| m.get("type").and_then(Value::as_str) == Some("user"))
                .and_then(|m| m.get("content").and_then(Value::as_str))
                .filter(|s| !s.trim().is_empty())
                .map(|s| truncate_summary(s, TITLE_MAX_CHARS))
        });

    let source_path = path.to_string_lossy().to_string();

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title: title.clone(),
        summary: title,
        project_dir: None, // populated later by scan_sessions_in_root
        created_at,
        last_active_at: last_active_at.or(created_at),
        source_path: Some(source_path),
        resume_command: Some(format!("gemini --resume {session_id}")),
        forked_from_id: None,
    })
}

fn move_session(source_path: &Path, dest_dir: &Path) -> Result<(), String> {
    move_single_file(source_path, dest_dir)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn parse_session_extracts_metadata() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-test.json");
        std::fs::write(
            &path,
            r#"{
              "sessionId": "gemini-session-123",
              "startTime": "2026-03-06T10:17:58.000Z",
              "lastUpdated": "2026-03-06T10:20:00.000Z",
              "messages": [
                {
                  "id": "msg-1",
                  "timestamp": "2026-03-06T10:17:58.000Z",
                  "type": "user",
                  "content": "hello world"
                }
              ]
            }"#,
        )
        .expect("write");

        let meta = parse_session(&path).expect("parse");
        assert_eq!(meta.provider_id, "gemini");
        assert_eq!(meta.session_id, "gemini-session-123");
        assert_eq!(meta.title.as_deref(), Some("hello world"));
        assert!(meta.created_at.is_some());
        assert!(meta.last_active_at.is_some());
        assert_eq!(
            meta.resume_command.as_deref(),
            Some("gemini --resume gemini-session-123")
        );
    }

    #[test]
    fn load_messages_maps_roles_and_skips_info_error() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.json");
        std::fs::write(
            &path,
            r#"{
              "sessionId": "test",
              "messages": [
                {"id":"1","timestamp":"2026-03-06T10:00:00Z","type":"user","content":"hello"},
                {"id":"2","timestamp":"2026-03-06T10:00:01Z","type":"gemini","content":"world"},
                {"id":"3","timestamp":"2026-03-06T10:00:02Z","type":"info","content":"system info"},
                {"id":"4","timestamp":"2026-03-06T10:00:03Z","type":"error","content":"MCP ERROR"}
              ]
            }"#,
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "world");
    }

    #[test]
    fn load_messages_handles_array_content() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.json");
        std::fs::write(
            &path,
            r#"{
              "sessionId": "test",
              "messages": [
                {"id":"1","timestamp":"2026-03-06T10:00:00Z","type":"user","content":[{"text":"hello"},{"text":"world"}]}
              ]
            }"#,
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "hello\nworld");
    }

    #[test]
    fn load_messages_includes_tool_calls() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.json");
        std::fs::write(
            &path,
            r#"{
              "sessionId": "test",
              "messages": [
                {
                  "id":"1",
                  "timestamp":"2026-03-10T08:24:50Z",
                  "type":"gemini",
                  "content":"",
                  "toolCalls":[{"id":"call_1","name":"web_search","args":{"query":"test"}}]
                },
                {
                  "id":"2",
                  "timestamp":"2026-03-10T08:25:00Z",
                  "type":"gemini",
                  "content":"Here are the results.",
                  "toolCalls":[{"id":"call_2","name":"web_fetch","args":{"url":"http://example.com"}}]
                }
              ]
            }"#,
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "assistant");
        assert!(msgs[0].content.contains("[Tool: web_search]"));
        assert_eq!(msgs[1].role, "assistant");
        assert!(msgs[1].content.contains("Here are the results."));
        assert!(msgs[1].content.contains("[Tool: web_fetch]"));
    }

    #[test]
    fn load_messages_skips_empty_content() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.json");
        std::fs::write(
            &path,
            r#"{
              "sessionId": "test",
              "messages": [
                {"id":"1","timestamp":"2026-03-06T10:00:00Z","type":"user","content":""},
                {"id":"2","timestamp":"2026-03-06T10:00:01Z","type":"gemini","content":"  "}
              ]
            }"#,
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 0);
    }

    #[test]
    fn validate_session_id_ok() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.json");
        std::fs::write(
            &path,
            r#"{
              "sessionId": "gemini-session-123",
              "startTime": "2026-03-06T10:17:58.000Z",
              "messages": [
                {"id":"1","timestamp":"2026-03-06T10:17:58.000Z","type":"user","content":"hi"}
              ]
            }"#,
        )
        .expect("write");

        let provider = GeminiProvider;
        assert!(provider
            .validate_session_id(&path, "gemini-session-123")
            .is_ok());
    }

    #[test]
    fn validate_session_id_mismatch() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.json");
        std::fs::write(
            &path,
            r#"{
              "sessionId": "gemini-session-123",
              "messages": []
            }"#,
        )
        .expect("write");

        let provider = GeminiProvider;
        assert!(provider.validate_session_id(&path, "wrong-id").is_err());
    }

    #[test]
    fn move_session_moves_file() {
        let temp = tempdir().expect("tempdir");
        let source_file = temp.path().join("session.json");
        std::fs::write(
            &source_file,
            r#"{"sessionId":"move-test","messages":[]}"#,
        )
        .expect("write");
        let dest_dir = temp.path().join("archived");
        let provider = GeminiProvider;
        provider
            .move_session(&source_file, &dest_dir)
            .expect("move should succeed");
        assert!(!source_file.exists(), "source file should be gone");
        assert!(
            dest_dir.join("session.json").exists(),
            "file should be at destination"
        );
    }

    #[test]
    fn load_raw_content_fallback_returns_none() {
        let provider = GeminiProvider;
        let result = provider
            .load_raw_content_fallback(Path::new("/tmp/fake.json"))
            .expect("should succeed");
        assert!(result.is_none());
    }

    #[test]
    fn scan_sessions_finds_session_files_in_project_structure() {
        let temp = tempdir().expect("tempdir");
        let project_dir = temp.path().join("my-project");
        let chats_dir = project_dir.join("chats");
        std::fs::create_dir_all(&chats_dir).expect("create chats dir");

        // Write .project_root
        std::fs::write(project_dir.join(".project_root"), "/home/user/my-project")
            .expect("write .project_root");

        // Write a session file
        std::fs::write(
            chats_dir.join("session-test.json"),
            r#"{
              "sessionId": "gemini-session-abc",
              "startTime": "2026-03-06T10:17:58.000Z",
              "messages": [
                {"id":"1","timestamp":"2026-03-06T10:17:58.000Z","type":"user","content":"test"}
              ]
            }"#,
        )
        .expect("write session");

        // Write a non-json file that should be ignored
        std::fs::write(chats_dir.join("notes.txt"), "not a session").expect("write notes");

        let sessions = scan_sessions_in_root(temp.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "gemini-session-abc");
        assert_eq!(
            sessions[0].project_dir.as_deref(),
            Some("/home/user/my-project")
        );
    }

    #[test]
    fn scan_sessions_empty_dir_returns_empty() {
        let temp = tempdir().expect("tempdir");
        let sessions = scan_sessions_in_root(temp.path());
        assert!(sessions.is_empty());
    }

    #[test]
    fn gemini_provider_trait_impl() {
        let provider = GeminiProvider;
        assert_eq!(provider.id(), "gemini");
        assert_eq!(provider.roots().len(), 2);
    }
}
