use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::fs_utils;
use crate::session_manager::{CumulativeTokenUsage, SessionMessage, SessionMeta, TokenUsage};

use super::utils::{
    extract_text, extract_tool_calls, extract_tool_results, infer_session_id_from_filename,
    move_single_file, parse_timestamp_to_ms, path_basename, push_raw_chunk, read_head_tail_lines,
    truncate_summary, TITLE_MAX_CHARS,
};
use super::SessionProvider;

const PROVIDER_ID: &str = "claude";

// ─── ClaudeProvider ─────────────────────────────────────────────────────────

/// Provider implementation for Claude Code sessions (.jsonl files in ~/.claude/projects/).
pub struct ClaudeProvider;

impl SessionProvider for ClaudeProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![
            crate::config::get_claude_projects_dir(),
            crate::config::get_claude_projects_archived_dir(),
        ]
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
        parse_session_at(path)
    }

    fn move_session(&self, source: &Path, dest: &Path) -> Result<(), String> {
        move_session(source, dest)
    }

    fn user_events(&self, path: &Path) -> Result<Vec<String>, String> {
        user_events_from_path(path)
    }

    fn user_events_with_uuid(&self, path: &Path) -> Result<Vec<(String, String)>, String> {
        user_events_with_uuid_from_path(path)
    }
}

// ─── Internal functions (unchanged logic) ───────────────────────────────────

pub fn scan_sessions_in_root(root: &Path) -> Vec<SessionMeta> {
    let files: Vec<std::path::PathBuf> = fs_utils::walk_jsonl_files(root)
        .into_iter()
        .map(|(path, _)| path)
        .collect();

    let mut sessions = Vec::new();
    for path in files {
        if let Some(meta) = parse_session(&path) {
            sessions.push(meta);
        }
    }

    sessions
}

pub fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
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

        if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
            continue;
        }

        let message = match value.get("message") {
            Some(message) => message,
            None => continue,
        };

        let mut role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        if role == "user" {
            if let Some(Value::Array(items)) = message.get("content") {
                let all_tool_results = !items.is_empty()
                    && items.iter().all(|item| {
                        item.get("type").and_then(Value::as_str) == Some("tool_result")
                    });
                if all_tool_results {
                    role = "tool".to_string();
                }
            }
        }

        let content_val = message.get("content");
        let content = content_val.map(extract_text).unwrap_or_default();
        if content.trim().is_empty() {
            continue;
        }
        let tool_result = content_val.and_then(extract_tool_results);
        let tool_calls = content_val.and_then(|v| {
            let calls = extract_tool_calls(v);
            if calls.is_empty() {
                None
            } else {
                Some(calls)
            }
        });

        let usage = if role == "assistant" {
            parse_token_usage(message.get("usage"))
        } else {
            None
        };
        if let Some(usage) = usage {
            cumulative_usage.add_usage(usage);
        }
        let cumulative_usage_for_message = usage.map(|_| cumulative_usage);

        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);
        messages.push(SessionMessage {
            role,
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

fn parse_token_usage(value: Option<&Value>) -> Option<TokenUsage> {
    let usage = value?;
    let parsed = TokenUsage {
        input_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_creation_input_tokens: usage
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read_input_tokens: usage
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    };

    (parsed.total() > 0).then_some(parsed)
}

pub fn load_raw_content_fallback(path: &Path) -> Result<Option<String>, String> {
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

        if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
            continue;
        }

        push_raw_chunk(&mut chunks, value.get("lastPrompt").and_then(Value::as_str));
        push_raw_chunk(&mut chunks, value.get("aiTitle").and_then(Value::as_str));

        if let Some(message) = value.get("message") {
            let content = message.get("content").map(extract_text).unwrap_or_default();
            push_raw_chunk(&mut chunks, Some(content.as_str()));
        }
    }

    if chunks.is_empty() {
        Ok(None)
    } else {
        Ok(Some(chunks.join("\n\n")))
    }
}

pub fn parse_session_at(path: &Path) -> Option<SessionMeta> {
    if is_agent_session(path) {
        return None;
    }
    parse_session(path)
}

/// Move a session (JSONL + sidecar) from one directory to another within the same filesystem.
/// Both source and destination are full paths (not relative to root).
pub fn move_session(source_path: &Path, dest_dir: &Path) -> Result<(), String> {
    // Move the JSONL file itself
    move_single_file(source_path, dest_dir)?;

    // Move sidecar directory if it exists
    if let Some(stem) = source_path.file_stem() {
        let source_sidecar = source_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(stem);
        if source_sidecar.exists() {
            let dest_sidecar = dest_dir.join(stem);
            if dest_sidecar.exists() {
                std::fs::remove_dir_all(&dest_sidecar).map_err(|e| {
                    format!("Failed to remove existing sidecar at destination: {e}")
                })?;
            }
            std::fs::rename(&source_sidecar, &dest_sidecar)
                .map_err(|e| format!("Failed to move sidecar directory: {e}"))?;
        }
    }

    Ok(())
}

fn parse_session(path: &Path) -> Option<SessionMeta> {
    if is_agent_session(path) {
        return None;
    }

    let (head, tail) = read_head_tail_lines(path, 10, 30).ok()?;

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut first_user_message: Option<String> = None;

    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if session_id.is_none() {
            session_id = value
                .get("sessionId")
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
        if first_user_message.is_none() {
            let is_user = value.get("type").and_then(Value::as_str) == Some("user")
                || value
                    .get("message")
                    .and_then(|m| m.get("role"))
                    .and_then(Value::as_str)
                    == Some("user");
            if is_user {
                if let Some(message) = value.get("message") {
                    let text = message.get("content").map(extract_text).unwrap_or_default();
                    let trimmed = text.trim();
                    if !trimmed.is_empty()
                        && !trimmed.contains("<local-command-caveat>")
                        && !trimmed.starts_with("<command-name>")
                    {
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

    let mut last_active_at: Option<i64> = None;
    let mut summary: Option<String> = None;
    let mut custom_title: Option<String> = None;

    for line in tail.iter().rev() {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if last_active_at.is_none() {
            last_active_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if custom_title.is_none()
            && value.get("type").and_then(Value::as_str) == Some("custom-title")
        {
            custom_title = value
                .get("customTitle")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
        }
        if summary.is_none() {
            if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            if let Some(message) = value.get("message") {
                let text = message.get("content").map(extract_text).unwrap_or_default();
                if !text.trim().is_empty() {
                    summary = Some(text);
                }
            }
        }
        if last_active_at.is_some() && summary.is_some() && custom_title.is_some() {
            break;
        }
    }

    let session_id_from_content = session_id.clone();
    let session_id = session_id.or_else(|| infer_session_id_from_filename(path));
    let session_id = session_id?;

    // Avoid false-positives: if sessionId was only inferred from the filename
    // (not found in the file content) and no other Claude-specific metadata
    // was found (like a top-level cwd field), this file is not a Claude session.
    // Prevents Claude from claiming Codex / other providers' session files.
    if session_id_from_content.is_none() && project_dir.is_none() {
        return None;
    }

    let title = custom_title
        .map(|t| truncate_summary(&t, TITLE_MAX_CHARS))
        .or_else(|| first_user_message.map(|t| truncate_summary(&t, TITLE_MAX_CHARS)))
        .or_else(|| project_dir.as_deref().and_then(path_basename));

    let summary = summary.map(|text| truncate_summary(&text, 160));

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title,
        summary,
        project_dir,
        created_at,
        last_active_at,
        source_path: Some(path.to_string_lossy().to_string()),
        resume_command: Some(format!("claude --resume {session_id}")),
        forked_from_id: None,
    })
}

/// Extract user input text events from a Claude session file.
/// Returns all user message texts in chronological order.
pub fn user_events_from_path(path: &Path) -> Result<Vec<String>, String> {
    collect_user_events(path, false, &[]).map(|events| events.into_iter().map(|(t, _)| t).collect())
}

/// Extract user events with UUIDs from a Claude session file.
/// Returns (text, uuid) pairs in chronological order.
/// The uuid is extracted from the top-level "uuid" field of each user event,
/// providing a strong cross-session matching signal for --resume detection.
pub fn user_events_with_uuid_from_path(path: &Path) -> Result<Vec<(String, String)>, String> {
    collect_user_events(path, false, &[])
}

/// Shared JSONL user-event collector for Claude-format session files.
///
/// Iterates all lines, filters for `type == "user"` events, optionally skips
/// `isMeta` events, and extracts user text + uuid from each.
///
/// `extra_block_types`: additional content block types to accept beyond "text"
/// (e.g. Qoder uses "input_text").
///
/// Returns `(text, uuid)` pairs in chronological order.
pub fn collect_user_events(
    path: &Path,
    skip_meta: bool,
    extra_block_types: &[&str],
) -> Result<Vec<(String, String)>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut events: Vec<(String, String)> = Vec::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }

        let obj: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Only process user events
        if obj["type"].as_str() != Some("user") {
            continue;
        }

        // Optionally skip isMeta events (Qoder system-internal messages)
        if skip_meta && obj.get("isMeta").and_then(Value::as_bool) == Some(true) {
            continue;
        }

        // Extract uuid — use empty string if missing
        let uuid = obj["uuid"].as_str().unwrap_or("").to_string();

        // Extract user text from message.content
        let text = extract_user_text_from_message(&obj["message"], extra_block_types);
        if !text.is_empty() {
            events.push((text, uuid));
        }
    }

    Ok(events)
}

/// Extract user text from a Claude/Qoder-format message value.
///
/// Handles array-format content (`[{type: "text", text: "..."}]`),
/// string-format content (legacy), and object-format content (`{text: "..."}`).
///
/// `extra_block_types`: additional block types to accept beyond "text"
/// (e.g. Qoder uses "input_text").
pub fn extract_user_text_from_message(msg: &Value, extra_block_types: &[&str]) -> String {
    let content = match msg.get("content") {
        Some(c) => c,
        None => return String::new(),
    };

    match content {
        Value::Array(blocks) => {
            for block in blocks {
                let block_type = block["type"].as_str().unwrap_or("");
                if block_type == "text" || extra_block_types.contains(&block_type) {
                    if let Some(text) = block["text"].as_str() {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            return trimmed.to_string();
                        }
                    }
                }
            }
            String::new()
        }
        Value::String(s) => s.clone(),
        Value::Object(map) => map
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

fn is_agent_session(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.starts_with("agent-"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn load_messages_tool_use_shows_as_assistant_and_tool_result_as_tool() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"name\":\"Write\"}]},\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"content\":\"File written\"}]},\"timestamp\":\"2026-03-06T10:00:01Z\"}\n",
            ),
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "assistant");
        assert!(msgs[0].content.contains("[Tool: Write]"));
        assert_eq!(msgs[1].role, "tool");
        assert!(msgs[1].content.contains("[Tool Result]"));
        assert!(msgs[1].tool_result.is_some());
        assert_eq!(
            msgs[1].tool_result.as_ref().unwrap().content,
            "File written"
        );
    }

    #[test]
    fn load_messages_parses_assistant_usage_and_cumulative_totals() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"message\":{\"role\":\"user\",\"content\":\"u1\"},\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"message\":{\"role\":\"assistant\",\"content\":\"a1\",\"usage\":{\"input_tokens\":10,\"cache_creation_input_tokens\":20,\"cache_read_input_tokens\":30,\"output_tokens\":40}},\"timestamp\":\"2026-03-06T10:00:01Z\"}\n",
                "{\"message\":{\"role\":\"assistant\",\"content\":\"a2\",\"usage\":{\"input_tokens\":1,\"cache_creation_input_tokens\":2,\"cache_read_input_tokens\":3,\"output_tokens\":4}},\"timestamp\":\"2026-03-06T10:00:02Z\"}\n",
            ),
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");

        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].usage, None);
        assert_eq!(msgs[0].cumulative_usage, None);
        assert_eq!(msgs[1].usage.map(|usage| usage.total()), Some(100));
        assert_eq!(
            msgs[1].cumulative_usage.map(|usage| (
                usage.input_tokens,
                usage.output_tokens,
                usage.total_tokens
            )),
            Some((60, 40, 100))
        );
        assert_eq!(msgs[2].usage.map(|usage| usage.total()), Some(10));
        assert_eq!(
            msgs[2].cumulative_usage.map(|usage| (
                usage.input_tokens,
                usage.output_tokens,
                usage.total_tokens
            )),
            Some((66, 44, 110))
        );
    }

    #[test]
    fn load_messages_ignores_missing_and_zero_usage() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"message\":{\"role\":\"assistant\",\"content\":\"a1\"},\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"message\":{\"role\":\"assistant\",\"content\":\"a2\",\"usage\":{\"input_tokens\":0,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":0,\"output_tokens\":0}},\"timestamp\":\"2026-03-06T10:00:01Z\"}\n",
            ),
        )
        .expect("write");

        let msgs = load_messages(&path).expect("load");

        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].usage, None);
        assert_eq!(msgs[0].cumulative_usage, None);
        assert_eq!(msgs[1].usage, None);
        assert_eq!(msgs[1].cumulative_usage, None);
    }

    #[test]
    fn parse_session_uses_first_user_message_as_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"session-abc\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"How do I deploy?\"},\"sessionId\":\"session-abc\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
        .expect("write");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("How do I deploy?"));
        assert_eq!(meta.provider_id, "claude");
        assert_eq!(
            meta.resume_command.as_deref(),
            Some("claude --resume session-abc")
        );
    }

    #[test]
    fn load_raw_content_fallback_reads_last_prompt_and_ai_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"last-prompt\",\"lastPrompt\":\"What is this project?\",\"sessionId\":\"s1\"}\n",
                "{\"type\":\"ai-title\",\"aiTitle\":\"Project overview\",\"sessionId\":\"s1\"}\n",
            ),
        )
        .expect("write");

        let content = load_raw_content_fallback(&path).expect("load");
        assert_eq!(
            content.as_deref(),
            Some("What is this project?\n\nProject overview")
        );
    }

    #[test]
    fn load_raw_content_fallback_ignores_meta_empty_invalid_and_duplicates() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "not json\n",
                "{\"isMeta\":true,\"lastPrompt\":\"hidden\"}\n",
                "{\"lastPrompt\":\"  \"}\n",
                "{\"lastPrompt\":\"repeat\"}\n",
                "{\"aiTitle\":\"repeat\"}\n",
            ),
        )
        .expect("write");

        let content = load_raw_content_fallback(&path).expect("load");
        assert_eq!(content.as_deref(), Some("repeat"));
    }

    #[test]
    fn load_raw_content_fallback_returns_none_for_no_visible_content() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "not json\n",
                "{\"isMeta\":true,\"lastPrompt\":\"hidden\"}\n",
                "{\"lastPrompt\":\"  \"}\n",
            ),
        )
        .expect("write");

        let content = load_raw_content_fallback(&path).expect("load");
        assert_eq!(content, None);
    }

    #[test]
    fn claude_provider_trait_impl() {
        let provider = ClaudeProvider;
        assert_eq!(provider.id(), "claude");
        assert_eq!(provider.roots().len(), 2);
    }

    #[test]
    fn validate_session_id_ok() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"session-abc\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"message\":{\"role\":\"user\",\"content\":\"hi\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
        .expect("write");

        let provider = ClaudeProvider;
        assert!(provider.validate_session_id(&path, "session-abc").is_ok());
    }

    #[test]
    fn user_events_extracts_user_text_from_array_content() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"s1\",\"cwd\":\"/tmp\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hello world\"}]}}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"second message\"}]}}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"response\"}}\n",
            ),
        )
        .expect("write");

        let provider = ClaudeProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], "hello world");
        assert_eq!(events[1], "second message");
    }

    #[test]
    fn user_events_skips_tool_result_events() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"s1\",\"cwd\":\"/tmp\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"content\":\"result\"}]}}\n",
            ),
        )
        .expect("write");

        let provider = ClaudeProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn user_events_handles_string_content() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"s1\",\"cwd\":\"/tmp\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"plain text\"}}\n",
            ),
        )
        .expect("write");

        let provider = ClaudeProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "plain text");
    }

    #[test]
    fn user_events_returns_first_text_block() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"s1\",\"cwd\":\"/tmp\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"content\":\"result\"},{\"type\":\"text\",\"text\":\"actual input\"}]}}\n",
            ),
        )
        .expect("write");

        let provider = ClaudeProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "actual input");
    }

    #[test]
    fn user_events_returns_empty_when_no_text_block() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"s1\",\"cwd\":\"/tmp\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"content\":\"result\"}]}}\n",
            ),
        )
        .expect("write");

        let provider = ClaudeProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn validate_session_id_mismatch() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"session-abc\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"message\":{\"role\":\"user\",\"content\":\"hi\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
        .expect("write");

        let provider = ClaudeProvider;
        assert!(provider.validate_session_id(&path, "wrong-id").is_err());
    }
}
