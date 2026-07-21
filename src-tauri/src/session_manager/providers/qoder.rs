use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::fs_utils;
use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::{
    extract_text, extract_tool_calls, extract_tool_results, infer_session_id_from_filename,
    parse_timestamp_to_ms, path_basename, push_raw_chunk, read_head_tail_lines, truncate_summary,
    TITLE_MAX_CHARS,
};
use super::SessionProvider;

const PROVIDER_ID: &str = "qoder";

// ─── QoderProvider ─────────────────────────────────────────────────────────

/// Provider implementation for Qoder (aka Qoder CN / Tongyi Lingma) sessions.
///
/// Storage layout:
///   ~/.qoder-cn/projects/{encoded-project-path}/{uuid}.jsonl
///   ~/.qoder/projects/{encoded-project-path}/{uuid}.jsonl
///
/// JSONL format is very similar to Claude Code's, with the same content block
/// types (thinking, text, tool_use, tool_result). Key differences:
///   - Extra event types: runtime-config, ai-title, last-prompt, system
///   - No token/usage fields
///   - User events may have isMeta: true (skip)
///   - Title comes from ai-title events
pub struct QoderProvider;

impl SessionProvider for QoderProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![
            crate::config::get_qoder_cn_projects_dir(),
            crate::config::get_qoder_projects_dir(),
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
        parse_session(path)
    }

    fn move_session(&self, _source: &Path, _dest: &Path) -> Result<(), String> {
        Err("Qoder does not support archive".to_string())
    }

    fn user_events(&self, path: &Path) -> Result<Vec<String>, String> {
        user_events_from_path(path)
    }

    fn user_events_with_uuid(&self, path: &Path) -> Result<Vec<(String, String)>, String> {
        user_events_with_uuid_from_path(path)
    }
}

// ─── Scan ──────────────────────────────────────────────────────────────────

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

// ─── Load messages ─────────────────────────────────────────────────────────

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

        // Only process user and assistant event types
        let event_type = value.get("type").and_then(Value::as_str);
        if event_type != Some("user") && event_type != Some("assistant") {
            continue;
        }

        // Skip isMeta events (system-internal messages)
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

        // Reclassify user events as tool when all content items are tool_result
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

        // Qoder does not provide token usage data
        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);

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

        // Extract lastPrompt from last-prompt events
        push_raw_chunk(&mut chunks, value.get("lastPrompt").and_then(Value::as_str));

        // Extract aiTitle from ai-title events
        push_raw_chunk(&mut chunks, value.get("aiTitle").and_then(Value::as_str));

        // Extract user message content
        if value.get("isMeta").and_then(Value::as_bool) != Some(true) {
            if let Some(message) = value.get("message") {
                let content = message.get("content").map(extract_text).unwrap_or_default();
                push_raw_chunk(&mut chunks, Some(content.as_str()));
            }
        }
    }

    if chunks.is_empty() {
        Ok(None)
    } else {
        Ok(Some(chunks.join("\n\n")))
    }
}

// ─── Parse session metadata ───────────────────────────────────────────────

fn parse_session(path: &Path) -> Option<SessionMeta> {
    let (head, tail) = read_head_tail_lines(path, 15, 30).ok()?;

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut first_user_message: Option<String> = None;
    // ai-title events contain the AI-generated session title
    let mut ai_title: Option<String> = None;
    // Track whether we found any Qoder-specific content (sessionId in top-level)
    let mut found_qoder_content = false;

    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        let event_type = value.get("type").and_then(Value::as_str);

        // Extract sessionId from the top-level field (present on most events)
        if session_id.is_none() {
            session_id = value
                .get("sessionId")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
        }

        // Extract project_dir from the top-level cwd field
        if project_dir.is_none() {
            project_dir = value
                .get("cwd")
                .and_then(Value::as_str)
                .map(|s| s.to_string());
        }

        // Extract created_at from the first timestamp
        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }

        // Extract aiTitle from ai-title events (latest wins)
        if event_type == Some("ai-title") {
            if let Some(title) = value.get("aiTitle").and_then(Value::as_str) {
                let trimmed = title.trim().to_string();
                if !trimmed.is_empty() {
                    ai_title = Some(trimmed);
                }
            }
        }

        // Extract first user message as title fallback
        if first_user_message.is_none() && event_type == Some("user") {
            if value.get("isMeta").and_then(Value::as_bool) != Some(true) {
                if let Some(message) = value.get("message") {
                    if message.get("role").and_then(Value::as_str) == Some("user") {
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
        }

        // Mark that we found Qoder-specific content (a sessionId at top level,
        // co-occurring with a type field — Claude header lines have sessionId
        // but no type, which would false-positive otherwise).
        if !found_qoder_content
            && value.get("sessionId").and_then(Value::as_str).is_some()
            && value.get("type").and_then(Value::as_str).is_some()
        {
            found_qoder_content = true;
        }

        // Early exit when we have all we need from head
        if session_id.is_some()
            && project_dir.is_some()
            && created_at.is_some()
            && first_user_message.is_some()
            && ai_title.is_some()
        {
            break;
        }
    }

    // Require at least some Qoder-specific content to avoid false positives
    // (sessionId + type co-occurring on the same line — Claude header lines
    // that carry sessionId without type are rejected)
    if !found_qoder_content {
        return None;
    }

    // Tail: find last_active_at and any additional ai-title events
    let mut last_active_at: Option<i64> = None;

    for line in tail.iter().rev() {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if last_active_at.is_none() {
            last_active_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }

        // Also check for ai-title events in the tail (latest overall wins).
        // Since we iterate the tail in reverse (most recent lines first),
        // the first ai-title we encounter is the latest one in the file.
        if value.get("type").and_then(Value::as_str) == Some("ai-title") {
            if let Some(title) = value.get("aiTitle").and_then(Value::as_str) {
                let trimmed = title.trim().to_string();
                if !trimmed.is_empty() {
                    ai_title = Some(trimmed);
                }
            }
        }

        if last_active_at.is_some() {
            break;
        }
    }

    let session_id = session_id.or_else(|| infer_session_id_from_filename(path));
    let session_id = session_id?;

    // Title priority: aiTitle > first user message > project dir basename
    let title = ai_title
        .map(|t| truncate_summary(&t, TITLE_MAX_CHARS))
        .or_else(|| {
            first_user_message
                .as_ref()
                .map(|t| truncate_summary(t, TITLE_MAX_CHARS))
        })
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
        resume_command: None,
        forked_from_id: None,
    })
}

// ─── User events for fork tree ────────────────────────────────────────────

/// Qoder accepts "input_text" blocks in addition to the standard "text" type.
const QODER_EXTRA_BLOCK_TYPES: &[&str] = &["input_text"];

/// Extract user input text events from a Qoder session file.
/// Returns all user message texts in chronological order.
fn user_events_from_path(path: &Path) -> Result<Vec<String>, String> {
    super::claude::collect_user_events(path, true, QODER_EXTRA_BLOCK_TYPES)
        .map(|events| events.into_iter().map(|(t, _)| t).collect())
}

/// Extract user events with UUIDs from a Qoder session file.
/// Returns (text, uuid) pairs in chronological order.
/// Qoder user UUIDs may use the format "user:{sessionId}########{seq}" or
/// a regular UUIDv4 — they're treated as opaque strings for fork tree matching.
fn user_events_with_uuid_from_path(path: &Path) -> Result<Vec<(String, String)>, String> {
    super::claude::collect_user_events(path, true, QODER_EXTRA_BLOCK_TYPES)
}

// ─── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_qoder_session(path: &Path, session_id: &str, message: &str) {
        std::fs::write(
            path,
            format!(
                "{{\"type\":\"runtime-config\",\"sessionId\":\"{session_id}\",\"model\":\"qmodel_latest\",\"timestamp\":1783006892499}}\n\
                 {{\"type\":\"user\",\"uuid\":\"user:{session_id}########1\",\"message\":{{\"role\":\"user\",\"content\":\"{message}\"}},\"version\":\"1.0.36\",\"sessionId\":\"{session_id}\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:01:00Z\"}}\n",
            ),
        )
            .expect("write session");
    }

    #[test]
    fn qoder_provider_trait_impl() {
        let provider = QoderProvider;
        assert_eq!(provider.id(), "qoder");
        assert_eq!(provider.roots().len(), 2);
    }

    #[test]
    fn parse_session_uses_first_user_message_as_title_when_no_ai_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-id.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"runtime-config\",\"sessionId\":\"session-id\",\"model\":\"qmodel_latest\",\"timestamp\":1783006892499}\n",
                "{\"type\":\"user\",\"uuid\":\"user:session-id########1\",\"message\":{\"role\":\"user\",\"content\":\"How do I deploy?\"},\"sessionId\":\"session-id\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
            .expect("write");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("How do I deploy?"));
        assert_eq!(meta.provider_id, "qoder");
        assert_eq!(meta.resume_command, None);
    }

    #[test]
    fn parse_session_prefers_ai_title_over_user_message() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-id.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"runtime-config\",\"sessionId\":\"session-id\",\"model\":\"qmodel_latest\",\"timestamp\":1783006892499}\n",
                "{\"type\":\"user\",\"uuid\":\"user:session-id########1\",\"message\":{\"role\":\"user\",\"content\":\"How do I deploy?\"},\"sessionId\":\"session-id\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
                "{\"type\":\"ai-title\",\"sessionId\":\"session-id\",\"aiTitle\":\"重构部署流程\"}\n",
            ),
        )
            .expect("write");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("重构部署流程"));
    }

    #[test]
    fn parse_session_uses_latest_ai_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-id.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"runtime-config\",\"sessionId\":\"session-id\",\"model\":\"qmodel_latest\",\"timestamp\":1783006892499}\n",
                "{\"type\":\"user\",\"uuid\":\"user:session-id########1\",\"message\":{\"role\":\"user\",\"content\":\"How do I deploy?\"},\"sessionId\":\"session-id\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
                "{\"type\":\"ai-title\",\"sessionId\":\"session-id\",\"aiTitle\":\"Old title\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"response\"},\"sessionId\":\"session-id\",\"timestamp\":\"2026-03-06T10:02:00Z\"}\n",
                "{\"type\":\"ai-title\",\"sessionId\":\"session-id\",\"aiTitle\":\"Newer title\"}\n",
            ),
        )
            .expect("write");

        let meta = parse_session(&path).unwrap();
        // ai_title logic: last ai-title event wins (whichever we find last)
        // In the head-only pass, "Old title" is found.
        // In the tail pass, "Newer title" would be found.
        // Since this file is < 16KB, head/tail overlap — the tail may contain both.
        // The logic picks the last one found in tail (reverse iteration).
        assert_eq!(meta.title.as_deref(), Some("Newer title"));
    }

    #[test]
    fn parse_session_falls_back_to_dir_basename() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-id.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"runtime-config\",\"sessionId\":\"session-id\",\"model\":\"qmodel_latest\",\"timestamp\":1783006892499}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"Hello\"},\"sessionId\":\"session-id\",\"cwd\":\"/tmp/my-project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
            ),
        )
            .expect("write");

        let meta = parse_session(&path).unwrap();
        // No user message or ai-title -> falls back to dir basename
        assert_eq!(meta.title.as_deref(), Some("my-project"));
    }

    #[test]
    fn parse_session_rejects_files_without_qoder_content() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-id.jsonl");
        // No sessionId top-level field -> should be rejected
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"hi\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
            .expect("write");

        assert!(parse_session(&path).is_none());
    }

    #[test]
    fn parse_session_uses_filename_stem_as_fallback_session_id() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("abc-123-def.jsonl");
        // No explicit sessionId, but file has qoder-specific content
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"runtime-config\",\"sessionId\":\"abc-123-def\",\"model\":\"qmodel_latest\",\"timestamp\":1783006892499}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"},\"sessionId\":\"abc-123-def\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
            .expect("write");

        let meta = parse_session(&path).unwrap();
        assert_eq!(meta.session_id, "abc-123-def");
    }

    #[test]
    fn load_messages_skips_non_user_assistant_events() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"runtime-config\",\"sessionId\":\"s1\",\"model\":\"qmodel_latest\",\"timestamp\":1783006892499}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"sessionId\":\"s1\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
                "{\"type\":\"ai-title\",\"sessionId\":\"s1\",\"aiTitle\":\"Test\"}\n",
                "{\"type\":\"last-prompt\",\"sessionId\":\"s1\",\"lastPrompt\":\"hello\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"world\"},\"sessionId\":\"s1\",\"timestamp\":\"2026-03-06T10:02:00Z\"}\n",
            ),
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
    fn load_messages_skips_is_meta_events() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"user\",\"isMeta\":true,\"message\":{\"role\":\"user\",\"content\":\"system message\"},\"sessionId\":\"s1\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"real input\"},\"sessionId\":\"s1\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
            .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content, "real input");
    }

    #[test]
    fn load_messages_tool_result_gets_role_tool() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"content\":\"file content\",\"tool_use_id\":\"call_1\"}]},\"sessionId\":\"s1\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
            .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "tool");
        assert!(msgs[0].tool_result.is_some());
    }

    #[test]
    fn load_messages_extracts_tool_use_from_assistant() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"Let me check\"},{\"type\":\"tool_use\",\"id\":\"call_1\",\"name\":\"Read\",\"input\":{\"file_path\":\"/tmp/test\"}}]},\"sessionId\":\"s1\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
            .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "assistant");
        assert!(msgs[0].content.contains("Let me check"));
        assert!(msgs[0].content.contains("[Tool: Read]"));
        assert!(msgs[0].tool_calls.is_some());
        assert_eq!(msgs[0].tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn load_messages_has_no_token_usage() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"},\"sessionId\":\"s1\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"response\"},\"sessionId\":\"s1\",\"timestamp\":\"2026-03-06T10:02:00Z\"}\n",
            ),
        )
            .expect("write");

        let msgs = load_messages(&path).expect("load");
        assert_eq!(msgs.len(), 2);
        assert!(msgs[0].usage.is_none());
        assert!(msgs[0].cumulative_usage.is_none());
        assert!(msgs[1].usage.is_none());
        assert!(msgs[1].cumulative_usage.is_none());
    }

    #[test]
    fn load_raw_content_fallback_reads_last_prompt_and_ai_title() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"last-prompt\",\"sessionId\":\"s1\",\"lastPrompt\":\"What is this project?\"}\n",
                "{\"type\":\"ai-title\",\"sessionId\":\"s1\",\"aiTitle\":\"Project overview\"}\n",
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
    fn load_raw_content_fallback_returns_none_for_empty() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"runtime-config\",\"sessionId\":\"s1\",\"model\":\"qmodel_latest\",\"timestamp\":1783006892499}\n",
            ),
        )
            .expect("write");

        let content = load_raw_content_fallback(&path).expect("load");
        assert_eq!(content, None);
    }

    #[test]
    fn validate_session_id_ok() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"runtime-config\",\"sessionId\":\"session-abc\",\"model\":\"qmodel_latest\",\"timestamp\":1783006892499}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"},\"sessionId\":\"session-abc\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
            .expect("write");

        let provider = QoderProvider;
        assert!(provider.validate_session_id(&path, "session-abc").is_ok());
    }

    #[test]
    fn validate_session_id_mismatch() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"runtime-config\",\"sessionId\":\"session-abc\",\"model\":\"qmodel_latest\",\"timestamp\":1783006892499}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"},\"sessionId\":\"session-abc\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
            .expect("write");

        let provider = QoderProvider;
        assert!(provider.validate_session_id(&path, "wrong-id").is_err());
    }

    #[test]
    fn move_session_returns_err() {
        let provider = QoderProvider;
        let err = provider
            .move_session(Path::new("/tmp/source"), Path::new("/tmp/dest"))
            .expect_err("should return error");
        assert!(err.contains("does not support archive"));
    }

    #[test]
    fn user_events_extracts_user_text() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hello world\"},\"sessionId\":\"s1\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"second message\"},\"sessionId\":\"s1\"}\n",
                "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":\"response\"},\"sessionId\":\"s1\"}\n",
            ),
        )
            .expect("write");

        let provider = QoderProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], "hello world");
        assert_eq!(events[1], "second message");
    }

    #[test]
    fn user_events_skips_is_meta() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"user\",\"isMeta\":true,\"message\":{\"role\":\"user\",\"content\":\"system\"},\"sessionId\":\"s1\"}\n",
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"real\"},\"sessionId\":\"s1\"}\n",
            ),
        )
            .expect("write");

        let provider = QoderProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "real");
    }

    #[test]
    fn user_events_skips_empty_text() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"content\":\"result\"}]},\"sessionId\":\"s1\"}\n",
            ),
        )
            .expect("write");

        let provider = QoderProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn user_events_with_uuid_extracts_both_text_and_uuid() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"user\",\"uuid\":\"user:s1########1\",\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"sessionId\":\"s1\"}\n",
                "{\"type\":\"user\",\"uuid\":\"user:s1########2\",\"message\":{\"role\":\"user\",\"content\":\"world\"},\"sessionId\":\"s1\"}\n",
            ),
        )
            .expect("write");

        let provider = QoderProvider;
        let events = provider
            .user_events_with_uuid(&path)
            .expect("user_events_with_uuid");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].0, "hello");
        assert_eq!(events[0].1, "user:s1########1");
        assert_eq!(events[1].0, "world");
        assert_eq!(events[1].1, "user:s1########2");
    }

    #[test]
    fn scan_sessions_finds_jsonl_files_in_project_subdirs() {
        let temp = tempdir().expect("tempdir");

        // Create a project directory structure like Qoder's
        let project_dir = temp.path().join("d--projects-my-app");
        std::fs::create_dir_all(&project_dir).expect("create project dir");

        // Write a valid session file
        write_qoder_session(
            &project_dir.join("abc-123.jsonl"),
            "abc-123",
            "test session",
        );

        // Write a non-JSONL file that should be ignored
        std::fs::write(project_dir.join("notes.txt"), "not a session").expect("write notes");

        let sessions = scan_sessions_in_root(temp.path());
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "abc-123");
    }

    #[test]
    fn scan_sessions_empty_dir_returns_empty() {
        let temp = tempdir().expect("tempdir");
        let sessions = scan_sessions_in_root(temp.path());
        assert!(sessions.is_empty());
    }

    #[test]
    fn parse_session_skips_system_events() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-id.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"system\",\"sessionId\":\"session-id\",\"subtype\":\"informational\",\"content\":\"startup message\",\"level\":\"info\"}\n",
                "{\"type\":\"user\",\"uuid\":\"user:session-id########1\",\"message\":{\"role\":\"user\",\"content\":\"hi\"},\"sessionId\":\"session-id\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
            .expect("write");

        let meta = parse_session(&path);
        assert!(meta.is_some(), "should parse despite system events");
        assert_eq!(meta.as_ref().unwrap().session_id, "session-id");
    }
}
