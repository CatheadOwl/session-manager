use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::fs_utils;
use crate::session_manager::types::{ToolCallInfo, ToolResultInfo};
use crate::session_manager::{CumulativeTokenUsage, SessionMessage, SessionMeta, TokenUsage};

use super::utils::{
    extract_text, infer_session_id_from_filename, move_single_file, parse_timestamp_to_ms,
    path_basename, push_raw_chunk, read_head_tail_lines, truncate_summary, truncate_tool_input,
    TITLE_MAX_CHARS,
};
use super::SessionProvider;

const PROVIDER_ID: &str = "pi";

// ─── PiProvider ────────────────────────────────────────────────────────────

/// Provider implementation for Pi (by Earendil Inc.) coding agent sessions.
///
/// Storage layout:
///   ~/.pi/agent/sessions/{encoded-project-path}/
///     └── {timestamp}_{uuid}.jsonl
///
/// JSONL format (v3): tree-structured entries with id/parentId.
/// Key differences from Claude Code:
///   - First line is a session header (type:"session")
///   - Entries use 8-char hex IDs + parentId tree
///   - Content blocks: "toolCall" + "arguments" instead of "tool_use" + "input"
///   - Tool results are separate message entries (role:"toolResult"), not inline
///   - Bash execution has its own role ("bashExecution")
///   - Usage/cost embedded in assistant message
///   - Extra entry types: model_change, thinking_level_change, compaction, etc.
pub struct PiProvider;

impl SessionProvider for PiProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![crate::config::get_pi_sessions_dir()]
    }

    fn scan_sessions(&self, root: &Path) -> Vec<SessionMeta> {
        scan_sessions_in_root(root)
    }

    fn load_messages(&self, path: &Path) -> Result<Vec<SessionMessage>, String> {
        load_messages(path)
    }

    fn load_raw_content_fallback(&self, path: &Path) -> Result<Option<String>, String> {
        load_raw_content_fallback(path)
    }

    fn parse_session(&self, path: &Path) -> Option<SessionMeta> {
        parse_session(path)
    }

    fn move_session(&self, _source: &Path, _dest: &Path) -> Result<(), String> {
        Err("Pi does not support archive".to_string())
    }

    fn user_events(&self, path: &Path) -> Result<Vec<String>, String> {
        pi_fork_view_unsupported(path)
    }

    fn user_events_with_uuid(&self, path: &Path) -> Result<Vec<(String, String)>, String> {
        pi_fork_view_unsupported(path)
    }
}

// ─── Scan ──────────────────────────────────────────────────────────────────

fn scan_sessions_in_root(root: &Path) -> Vec<SessionMeta> {
    let files: Vec<PathBuf> = fs_utils::walk_jsonl_files(root)
        .into_iter()
        .map(|(path, _)| path)
        .collect();

    let mut sessions = Vec::new();
    for path in files {
        // Skip sub-run session files (always named "session.jsonl" inside run-N/)
        if path.file_name().and_then(|n| n.to_str()) == Some("session.jsonl") {
            continue;
        }
        if let Some(meta) = parse_session(&path) {
            sessions.push(meta);
        }
    }

    sessions
}

// ─── Load messages ─────────────────────────────────────────────────────────

fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut cumulative_usage = CumulativeTokenUsage::default();

    for line in reader.lines() {
        let line = match line {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        // Skip session header and non-message entries
        let entry_type = value.get("type").and_then(Value::as_str);
        if entry_type != Some("message") {
            continue;
        }

        let message = match value.get("message") {
            Some(m) => m,
            None => continue,
        };

        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown");

        // Map Pi roles to unified roles
        let mapped_role = match role {
            "user" => "user",
            "assistant" => "assistant",
            "toolResult" => "tool",
            "bashExecution" => "tool",
            _ => continue, // Skip custom, branchSummary, compactionSummary
        };

        // Extract content based on role
        let (content, tool_result) = match role {
            "bashExecution" => {
                // bashExecution uses "output" field instead of "content"
                let cmd = message.get("command").and_then(Value::as_str).unwrap_or("");
                let output = message.get("output").and_then(Value::as_str).unwrap_or("");
                let full = if !cmd.is_empty() {
                    format!("$ {cmd}\n{output}")
                } else {
                    output.to_string()
                };
                let trimmed = full.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }
                (trimmed, None)
            }
            "toolResult" => {
                // toolResult has toolCallId + content array
                let content_val = message.get("content");
                let text = content_val.map(extract_text).unwrap_or_default();
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }
                let tool_call_id = message
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                let tool_result_info = ToolResultInfo {
                    content: trimmed.clone(),
                    call_id: tool_call_id,
                };
                (trimmed, Some(tool_result_info))
            }
            _ => {
                // user / assistant — extract from content array or string
                let content_val = message.get("content");
                let text = content_val.map(extract_text).unwrap_or_default();
                let trimmed = text.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }
                (trimmed, None)
            }
        };

        // Extract tool calls from assistant messages (Pi uses "toolCall" type)
        let tool_calls = if role == "assistant" {
            let calls = extract_pi_tool_calls(message.get("content"));
            if calls.is_empty() {
                None
            } else {
                Some(calls)
            }
        } else {
            None
        };

        // Parse usage from assistant messages
        let usage = if role == "assistant" {
            parse_pi_token_usage(message.get("usage"))
        } else {
            None
        };
        if let Some(u) = usage {
            cumulative_usage.add_usage(u);
        }
        let cumulative_usage_for_message = usage.map(|_| cumulative_usage);

        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);

        messages.push(SessionMessage {
            role: mapped_role.to_string(),
            content,
            ts,
            usage,
            cumulative_usage: cumulative_usage_for_message,
            tool_calls,
            tool_result,
        });
    }

    Ok(messages)
}

/// Parse Pi's usage format into TokenUsage.
///
/// Pi usage format:
/// ```json
/// {"input": 9180, "output": 101, "cacheRead": 0, "cacheWrite": 0, "reasoning": 50, "totalTokens": 9281}
/// ```
fn parse_pi_token_usage(value: Option<&Value>) -> Option<TokenUsage> {
    let usage = value?;
    let parsed = TokenUsage {
        input_tokens: usage.get("input").and_then(Value::as_u64).unwrap_or(0),
        cache_creation_input_tokens: usage.get("cacheWrite").and_then(Value::as_u64).unwrap_or(0),
        cache_read_input_tokens: usage.get("cacheRead").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: usage.get("output").and_then(Value::as_u64).unwrap_or(0),
    };

    (parsed.total() > 0).then_some(parsed)
}

/// Extract tool call info from Pi content blocks.
///
/// Pi uses `toolCall` type with `arguments` (not Claude's `tool_use` + `input`):
/// ```json
/// {"type": "toolCall", "id": "call_00_xxx", "name": "bash", "arguments": {"command": "ls"}}
/// ```
fn extract_pi_tool_calls(content: Option<&Value>) -> Vec<ToolCallInfo> {
    let items = match content {
        Some(Value::Array(items)) => items,
        _ => return Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            let item_type = item.get("type").and_then(Value::as_str)?;
            if item_type != "toolCall" {
                return None;
            }
            let name = item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let input = item
                .get("arguments")
                .map(truncate_tool_input)
                .unwrap_or_default();
            let call_id = item
                .get("id")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
            Some(ToolCallInfo {
                name,
                input,
                call_id,
            })
        })
        .collect()
}

// ─── Raw content fallback ─────────────────────────────────────────────────

fn load_raw_content_fallback(path: &Path) -> Result<Option<String>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut chunks: Vec<String> = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        let entry_type = value.get("type").and_then(Value::as_str);
        if entry_type != Some("message") {
            continue;
        }

        let Some(message) = value.get("message") else {
            continue;
        };
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");

        match role {
            "user" => {
                let content = message.get("content").map(extract_text).unwrap_or_default();
                push_raw_chunk(&mut chunks, Some(content.as_str()));
            }
            "bashExecution" => {
                let text = message.get("output").and_then(Value::as_str).unwrap_or("");
                push_raw_chunk(&mut chunks, Some(text));
            }
            _ => {}
        }
    }

    if chunks.is_empty() {
        Ok(None)
    } else {
        Ok(Some(chunks.join("\n\n")))
    }
}

// ─── Parse session metadata ────────────────────────────────────────────────

fn parse_session(path: &Path) -> Option<SessionMeta> {
    let (head, tail) = read_head_tail_lines(path, 15, 30).ok()?;

    // First line must be Pi session header: {"type":"session","version":3,...}
    let first_line = head.first()?;
    let header: Value = serde_json::from_str(first_line).ok()?;
    if header.get("type").and_then(Value::as_str) != Some("session") {
        return None;
    }

    let session_id = header
        .get("id")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let project_dir = header
        .get("cwd")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let created_at = header.get("timestamp").and_then(parse_timestamp_to_ms);

    // Read parentSession for fork detection (/fork, /clone create this)
    // Value is a file path like ".../{timestamp}_{parent-uuid}.jsonl"
    let forked_from_id = header
        .get("parentSession")
        .and_then(Value::as_str)
        .and_then(|p| Path::new(p).file_stem())
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.rsplit('_').next())
        .map(|s| s.to_string());

    // Also try filename fallback for session_id if header id is missing
    let session_id = session_id.or_else(|| infer_session_id_from_filename(path));
    let session_id = session_id?;

    // Scan head for first user message (title candidate)
    let mut first_user_message: Option<String> = None;

    for line in head.iter().skip(1) {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if first_user_message.is_some() {
            break;
        }

        if value.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }

        let Some(message) = value.get("message") else {
            continue;
        };
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }

        let text = message.get("content").map(extract_text).unwrap_or_default();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            first_user_message = Some(trimmed.to_string());
        }
    }

    // Scan tail for last_active_at
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

    // Title: first user message > project dir basename
    let title = first_user_message
        .as_ref()
        .map(|t| truncate_summary(t, TITLE_MAX_CHARS))
        .or_else(|| project_dir.as_deref().and_then(path_basename));

    let summary = first_user_message
        .as_ref()
        .map(|text| truncate_summary(text, 160));

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title,
        summary,
        project_dir,
        created_at,
        last_active_at,
        source_path: Some(path.to_string_lossy().to_string()),
        resume_command: Some(format!("pi --resume {session_id}")),
        forked_from_id,
    })
}

// ─── Move session ─────────────────────────────────────────────────────────

/// Physical file move logic — kept for when archive support is added.
/// Currently unused because Pi does not support archive/restore operations.
#[allow(dead_code)]
fn move_pi_session(source: &Path, dest: &Path) -> Result<(), String> {
    move_single_file(source, dest)?;

    // Move sidecar directory if it exists (the {session} dir alongside the .jsonl file)
    // Pi stores sub-run data in a directory named after the session file (without .jsonl ext)
    if let Some(stem) = source.file_stem() {
        let source_sidecar = source.parent().unwrap_or_else(|| Path::new("")).join(stem);
        if source_sidecar.exists() {
            let dest_sidecar = dest.join(stem);
            if dest_sidecar.exists() {
                std::fs::remove_dir_all(&dest_sidecar).map_err(|e| {
                    format!("Failed to remove existing sidecar at destination: {e}")
                })?;
            }
            std::fs::rename(&source_sidecar, &dest_sidecar)
                .map_err(|e| format!("Failed to move Pi sidecar directory: {e}"))?;
        }
    }

    Ok(())
}

// ─── Fork tree support ────────────────────────────────────────────────────

fn pi_fork_view_unsupported<T>(_path: &Path) -> Result<T, String> {
    Err("Pi fork view is not supported yet".to_string())
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Helper: write a minimal Pi session file with header + messages.
    fn write_pi_session(path: &Path, session_id: &str, cwd: &str, lines: &[&str]) {
        let mut content = format!(
            r#"{{"type":"session","version":3,"id":"{session_id}","timestamp":"2026-07-30T06:19:36.428Z","cwd":"{cwd}"}}"#,
        );
        for line in lines {
            content.push('\n');
            content.push_str(line);
        }
        std::fs::write(path, content).expect("write session");
    }

    #[test]
    fn pi_provider_trait_impl() {
        let provider = PiProvider;
        assert_eq!(provider.id(), "pi");
        assert_eq!(provider.roots().len(), 1);
    }

    #[test]
    fn parse_session_from_header() {
        let temp = tempdir().expect("tempdir");
        let path = temp
            .path()
            .join("019fb1ad-aa6c-7d65-b92d-24d5547df15f.jsonl");
        write_pi_session(
            &path,
            "019fb1ad-aa6c-7d65-b92d-24d5547df15f",
            "/tmp/my-project",
            &[],
        );

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.session_id, "019fb1ad-aa6c-7d65-b92d-24d5547df15f");
        assert_eq!(meta.provider_id, "pi");
        assert_eq!(meta.project_dir.as_deref(), Some("/tmp/my-project"));
        assert_eq!(
            meta.resume_command.as_deref(),
            Some("pi --resume 019fb1ad-aa6c-7d65-b92d-24d5547df15f")
        );
        assert_eq!(meta.forked_from_id, None);
    }

    #[test]
    fn parse_session_with_parent_session() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("child-uuid-123.jsonl");

        // Session header with parentSession pointing to parent session file
        let header = r#"{"type":"session","version":3,"id":"child-uuid-123","timestamp":"2026-07-30T07:00:00Z","cwd":"/tmp","parentSession":"~/.pi/agent/sessions/proj/2026-07-30T06-00-00Z_parent-uuid-abc.jsonl"}"#;
        std::fs::write(&path, header).expect("write session");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.session_id, "child-uuid-123");
        assert_eq!(meta.forked_from_id.as_deref(), Some("parent-uuid-abc"));
    }

    #[test]
    fn parse_session_rejects_non_pi_header() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        // Claude-style header (no type:"session") — should be rejected
        std::fs::write(
            &path,
            r#"{"sessionId":"abc","cwd":"/tmp","timestamp":"2026-03-06T10:00:00Z"}"#,
        )
        .expect("write");

        assert!(parse_session(&path).is_none());
    }

    #[test]
    fn parse_session_uses_first_user_message_as_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "abc-123",
            "/tmp/project",
            &[
                r#"{"type":"message","id":"a1b2c3d4","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"How do I deploy this project?"}}"#,
            ],
        );

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("How do I deploy this project?"));
    }

    #[test]
    fn parse_session_falls_back_to_dir_basename() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "abc-123",
            "/tmp/my-project",
            &[
                r#"{"type":"message","id":"a1b2c3d4","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}"#,
            ],
        );

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("my-project"));
    }

    #[test]
    fn load_messages_user_and_assistant() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"message","id":"10000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"Hello"}}"#,
                r#"{"type":"message","id":"10000002","parentId":"10000001","timestamp":"2026-07-30T06:20:10Z","message":{"role":"assistant","content":[{"type":"text","text":"Hi there!"}]}}"#,
            ],
        );

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "Hello");
        assert_eq!(msgs[1].role, "assistant");
        assert_eq!(msgs[1].content, "Hi there!");
    }

    #[test]
    fn load_messages_skips_session_header_and_metadata() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"model_change","id":"m1000001","parentId":null,"timestamp":"2026-07-30T06:19:37Z","provider":"deepseek","modelId":"deepseek-v4-pro"}"#,
                r#"{"type":"thinking_level_change","id":"t1000001","parentId":"m1000001","timestamp":"2026-07-30T06:19:38Z","thinkingLevel":"high"}"#,
                r#"{"type":"message","id":"10000001","parentId":"t1000001","timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"Hello"}}"#,
                r#"{"type":"message","id":"10000002","parentId":"10000001","timestamp":"2026-07-30T06:20:10Z","message":{"role":"assistant","content":[{"type":"text","text":"World"}]}}"#,
            ],
        );

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn load_messages_tool_call_and_tool_result() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"message","id":"10000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"list files"}}"#,
                r#"{"type":"message","id":"10000002","parentId":"10000001","timestamp":"2026-07-30T06:20:10Z","message":{"role":"assistant","content":[{"type":"text","text":"Running ls"},{"type":"toolCall","id":"call_001","name":"bash","arguments":{"command":"ls"}}]}}"#,
                r#"{"type":"message","id":"10000003","parentId":"10000002","timestamp":"2026-07-30T06:20:20Z","message":{"role":"toolResult","toolCallId":"call_001","toolName":"bash","content":[{"type":"text","text":"file1.txt\nfile2.txt"}],"isError":false}}"#,
                r#"{"type":"message","id":"10000004","parentId":"10000003","timestamp":"2026-07-30T06:20:30Z","message":{"role":"assistant","content":[{"type":"text","text":"Done!"}]}}"#,
            ],
        );

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 4);

        // User
        assert_eq!(msgs[0].role, "user");

        // Assistant with tool call
        assert_eq!(msgs[1].role, "assistant");
        assert!(msgs[1].content.contains("Running ls"));
        assert!(msgs[1].content.contains("[Tool: bash]"));
        assert!(msgs[1].tool_calls.is_some());
        let calls = msgs[1].tool_calls.as_ref().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[0].call_id.as_deref(), Some("call_001"));

        // Tool result
        assert_eq!(msgs[2].role, "tool");
        assert!(msgs[2].content.contains("file1.txt"));
        assert!(msgs[2].tool_result.is_some());
        assert_eq!(
            msgs[2].tool_result.as_ref().unwrap().call_id,
            Some("call_001".to_string())
        );

        // Final assistant
        assert_eq!(msgs[3].role, "assistant");
        assert!(msgs[3].content.contains("Done!"));
    }

    #[test]
    fn load_messages_bash_execution() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"message","id":"10000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"run tests"}}"#,
                r#"{"type":"message","id":"10000002","parentId":"10000001","timestamp":"2026-07-30T06:20:10Z","message":{"role":"bashExecution","command":"cargo test","output":"test result: ok. 3 passed","exitCode":0,"cancelled":false,"truncated":false}}"#,
            ],
        );

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);

        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "run tests");

        assert_eq!(msgs[1].role, "tool");
        assert!(msgs[1].content.contains("cargo test"));
        assert!(msgs[1].content.contains("test result: ok. 3 passed"));
    }

    #[test]
    fn load_messages_skips_unknown_roles() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"message","id":"10000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"branchSummary","summary":"explored approach A","fromId":"prev0001"}}"#,
                r#"{"type":"message","id":"10000002","parentId":"10000001","timestamp":"2026-07-30T06:20:10Z","message":{"role":"compactionSummary","summary":"compressed earlier context","tokensBefore":50000}}"#,
                r#"{"type":"message","id":"10000003","parentId":"10000002","timestamp":"2026-07-30T06:21:00Z","message":{"role":"user","content":"real message"}}"#,
            ],
        );

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "real message");
    }

    #[test]
    fn load_messages_parses_usage() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"message","id":"10000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"hello"}}"#,
                r#"{"type":"message","id":"10000002","parentId":"10000001","timestamp":"2026-07-30T06:20:10Z","message":{"role":"assistant","content":[{"type":"text","text":"world"}],"usage":{"input":100,"output":20,"cacheRead":10,"cacheWrite":5,"totalTokens":135,"cost":{"total":0.001}}}}"#,
            ],
        );

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].usage.is_none());

        let usage = msgs[1].usage.expect("usage");
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 20);
        assert_eq!(usage.cache_read_input_tokens, 10);
        assert_eq!(usage.cache_creation_input_tokens, 5);
        assert_eq!(usage.input_total(), 115);
        assert_eq!(usage.total(), 135);

        let cumulative = msgs[1].cumulative_usage.expect("cumulative");
        assert_eq!(cumulative.input_tokens, 115);
        assert_eq!(cumulative.output_tokens, 20);
        assert_eq!(cumulative.total_tokens, 135);
    }

    #[test]
    fn load_messages_handles_user_content_as_string() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"message","id":"10000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"plain string message"}}"#,
            ],
        );

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "plain string message");
    }

    #[test]
    fn load_messages_skips_empty_content() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"message","id":"10000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"  "}}"#,
                r#"{"type":"message","id":"10000002","parentId":"10000001","timestamp":"2026-07-30T06:20:10Z","message":{"role":"assistant","content":[{"type":"text","text":""}]}}"#,
            ],
        );

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 0);
    }

    #[test]
    fn load_raw_content_fallback_collects_user_text() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"message","id":"10000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"first question"}}"#,
                r#"{"type":"message","id":"10000002","parentId":"10000001","timestamp":"2026-07-30T06:20:10Z","message":{"role":"assistant","content":[{"type":"text","text":"first answer"}]}}"#,
                r#"{"type":"message","id":"10000003","parentId":"10000002","timestamp":"2026-07-30T06:20:20Z","message":{"role":"user","content":"second question"}}"#,
            ],
        );

        let content = load_raw_content_fallback(&path).expect("load");
        assert_eq!(
            content.as_deref(),
            Some("first question\n\nsecond question")
        );
    }

    #[test]
    fn load_raw_content_fallback_includes_bash_execution() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"message","id":"10000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"run build"}}"#,
                r#"{"type":"message","id":"10000002","parentId":"10000001","timestamp":"2026-07-30T06:20:10Z","message":{"role":"bashExecution","command":"make","output":"Build succeeded"}}"#,
            ],
        );

        let content = load_raw_content_fallback(&path).expect("load");
        assert!(content.as_deref().unwrap().contains("run build"));
        assert!(content.as_deref().unwrap().contains("Build succeeded"));
    }

    #[test]
    fn load_raw_content_fallback_returns_none_for_empty() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(&path, "s1", "/tmp", &[]);

        let content = load_raw_content_fallback(&path).expect("load");
        assert_eq!(content, None);
    }

    #[test]
    fn move_session_returns_err_for_archive() {
        let provider = PiProvider;
        let err = provider
            .move_session(Path::new("/tmp/source.jsonl"), Path::new("/tmp/dest"))
            .expect_err("should return error");
        assert!(err.contains("does not support archive"));
    }

    #[test]
    fn fork_events_are_explicitly_unsupported() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        write_pi_session(
            &path,
            "s1",
            "/tmp",
            &[
                r#"{"type":"message","id":"a1000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"first msg"}}"#,
            ],
        );

        let provider = PiProvider;
        let err = provider
            .user_events(&path)
            .expect_err("fork events should be unsupported");
        assert!(err.contains("fork view is not supported"));

        let err = provider
            .user_events_with_uuid(&path)
            .expect_err("fork events with uuid should be unsupported");
        assert!(err.contains("fork view is not supported"));
    }

    #[test]
    fn validate_session_id_ok() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        write_pi_session(
            &path,
            "session-abc",
            "/tmp",
            &[
                r#"{"type":"message","id":"a1000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"hi"}}"#,
            ],
        );

        let provider = PiProvider;
        assert!(provider.validate_session_id(&path, "session-abc").is_ok());
    }

    #[test]
    fn validate_session_id_mismatch() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        write_pi_session(
            &path,
            "session-abc",
            "/tmp",
            &[
                r#"{"type":"message","id":"a1000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"hi"}}"#,
            ],
        );

        let provider = PiProvider;
        assert!(provider.validate_session_id(&path, "wrong-id").is_err());
    }

    #[test]
    fn scan_sessions_finds_jsonl_files_in_project_subdirs() {
        let temp = tempdir().expect("tempdir");

        // Simulate Pi directory structure: sessions/{encoded-path}/{session}.jsonl
        let project_dir = temp.path().join("--C--Users-o0lbh--");
        std::fs::create_dir_all(&project_dir).expect("create project dir");

        write_pi_session(
            &project_dir.join("2026-07-30T06-19-36-428Z_session-abc.jsonl"),
            "session-abc",
            "/tmp",
            &[
                r#"{"type":"message","id":"a1000001","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"hello"}}"#,
            ],
        );

        // Write a non-Pi .jsonl file that should still be rejected by parse_session
        std::fs::write(
            project_dir.join("other.jsonl"),
            r#"{"message":{"role":"user","content":"hi"}}"#,
        )
        .expect("write");

        let sessions = scan_sessions_in_root(temp.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "session-abc");
    }

    #[test]
    fn scan_sessions_skips_sub_run_session_jsonl_files() {
        let temp = tempdir().expect("tempdir");

        let project_dir = temp.path().join("--project--");
        std::fs::create_dir_all(&project_dir).expect("create project dir");

        // Regular session file
        write_pi_session(
            &project_dir.join("2026-07-30T06-00-00Z_sid-123.jsonl"),
            "sid-123",
            "/tmp",
            &[
                r#"{"type":"message","id":"a1","parentId":null,"timestamp":"2026-07-30T06:20:00Z","message":{"role":"user","content":"hello"}}"#,
            ],
        );

        // Sub-run session.jsonl (should be skipped)
        let run_dir = project_dir.join("sid-123").join("abc123").join("run-0");
        std::fs::create_dir_all(&run_dir).expect("create run dir");
        std::fs::write(
            run_dir.join("session.jsonl"),
            r#"{"version":1,"recordType":"message"}"#,
        )
        .expect("write");

        let sessions = scan_sessions_in_root(temp.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "sid-123");
    }
}
