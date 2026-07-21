use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::session_manager::types::ToolResultInfo;
use crate::session_manager::{SessionMessage, SessionMeta, ToolCallInfo};

use super::utils::{
    extract_text, move_single_file, parse_timestamp_to_ms, path_basename, read_head_tail_lines,
    truncate_summary, TITLE_MAX_CHARS, TOOL_CALL_INPUT_MAX_CHARS,
};
use super::SessionProvider;

const PROVIDER_ID: &str = "codex";
const CODEX_SESSION_INDEX_FILENAME: &str = "session_index.jsonl";
const VSCODE_CONTEXT_PREFIX: &str = "# Context from my IDE setup:";
const CODEX_REQUEST_MARKER: &str = "my request for codex";

#[derive(Deserialize)]
struct SessionIndexEntry {
    id: String,
    thread_name: String,
}

// ─── CodexProvider ──────────────────────────────────────────────────────────

/// Provider implementation for Codex / Cursor CLI sessions (.jsonl files in ~/.codex/).
pub struct CodexProvider;

impl SessionProvider for CodexProvider {
    fn id(&self) -> &str {
        PROVIDER_ID
    }

    fn roots(&self) -> Vec<PathBuf> {
        vec![
            crate::config::get_codex_sessions_dir(),
            crate::config::get_codex_archive_dir(),
        ]
    }

    fn scan_sessions(&self, root: &Path) -> Vec<SessionMeta> {
        let mut files = Vec::new();
        collect_jsonl_files(root, &mut files);

        let thread_titles = load_thread_titles();

        let mut sessions = Vec::new();
        for path in files {
            if let Some(meta) = parse_session_with_titles(&path, &thread_titles) {
                sessions.push(meta);
            }
        }
        sessions
    }

    fn parse_session(&self, path: &Path) -> Option<SessionMeta> {
        let thread_titles = load_thread_titles();
        parse_session_with_titles(path, &thread_titles)
    }

    fn load_messages(&self, path: &Path) -> Result<Vec<SessionMessage>, String> {
        load_messages(path)
    }

    fn load_raw_content_fallback(&self, _path: &Path) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn move_session(&self, source: &Path, dest: &Path) -> Result<(), String> {
        move_session(source, dest)
    }

    fn user_events(&self, path: &Path) -> Result<Vec<String>, String> {
        user_events_from_path(path)
    }
}

// ─── Scan helpers ───────────────────────────────────────────────────────────

/// Recursively walk a directory and collect all `.jsonl` file paths.
fn collect_jsonl_files(root: &Path, files: &mut Vec<PathBuf>) {
    if !root.exists() {
        return;
    }

    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_jsonl_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
            files.push(path);
        }
    }
}

// ─── User events for fork tree ──────────────────────────────────────────────

/// Extract user input text events from a Codex session file.
/// Returns all user message texts in chronological order.
fn user_events_from_path(path: &Path) -> Result<Vec<String>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut events: Vec<String> = Vec::new();

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

        // Only process response_item events
        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }

        let payload = match value.get("payload") {
            Some(p) => p,
            None => continue,
        };

        // Only user messages
        if payload.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        if payload.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }

        let text = payload.get("content").map(extract_text).unwrap_or_default();
        if !text.trim().is_empty() {
            events.push(text.trim().to_string());
        }
    }

    Ok(events)
}

// ─── Thread titles from session_index.jsonl ─────────────────────────────────

fn load_thread_titles() -> HashMap<String, String> {
    let index_path = crate::config::get_codex_dir().join(CODEX_SESSION_INDEX_FILENAME);
    if !index_path.exists() {
        return HashMap::new();
    }

    let file = match File::open(&index_path) {
        Ok(file) => file,
        Err(_) => return HashMap::new(),
    };

    let reader = BufReader::new(file);
    let mut titles = HashMap::new();
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => continue,
        };
        let Ok(entry) = serde_json::from_str::<SessionIndexEntry>(line.trim()) else {
            continue;
        };
        let id = entry.id.trim().to_string();
        let title = entry.thread_name.trim().to_string();
        if !id.is_empty() && !title.is_empty() {
            titles.insert(id, title);
        }
    }
    titles
}

// ─── Parse session metadata ─────────────────────────────────────────────────

fn parse_session_with_titles(
    path: &Path,
    thread_titles: &HashMap<String, String>,
) -> Option<SessionMeta> {
    let (head, tail) = read_head_tail_lines(path, 10, 30).ok()?;

    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<i64> = None;
    let mut first_user_message: Option<String> = None;
    let mut forked_from_id: Option<String> = None;

    // Extract metadata and first user message from head lines
    for line in &head {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if created_at.is_none() {
            created_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if value.get("type").and_then(Value::as_str) == Some("session_meta") {
            if let Some(payload) = value.get("payload") {
                if is_subagent_source(payload.get("source")) {
                    return None;
                }
                if session_id.is_none() {
                    session_id = payload
                        .get("id")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                }
                if forked_from_id.is_none() {
                    forked_from_id = payload
                        .get("forked_from_id")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                }
                if project_dir.is_none() {
                    project_dir = payload
                        .get("cwd")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                }
                if let Some(ts) = payload.get("timestamp").and_then(parse_timestamp_to_ms) {
                    created_at.get_or_insert(ts);
                }
            }
        }
        // Extract first user message as title candidate
        if first_user_message.is_none()
            && value.get("type").and_then(Value::as_str) == Some("response_item")
        {
            if let Some(payload) = value.get("payload") {
                if payload.get("type").and_then(Value::as_str) == Some("message")
                    && payload.get("role").and_then(Value::as_str) == Some("user")
                {
                    let text = payload.get("content").map(extract_text).unwrap_or_default();
                    if let Some(title) = title_candidate_from_user_message(&text) {
                        first_user_message = Some(title);
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

    // Extract last_active_at and summary from tail lines (reverse order)
    let mut last_active_at: Option<i64> = None;
    let mut summary: Option<String> = None;

    for line in tail.iter().rev() {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if last_active_at.is_none() {
            last_active_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if summary.is_none() && value.get("type").and_then(Value::as_str) == Some("response_item") {
            if let Some(payload) = value.get("payload") {
                if payload.get("type").and_then(Value::as_str) == Some("message") {
                    let text = payload.get("content").map(extract_text).unwrap_or_default();
                    if !text.trim().is_empty() {
                        summary = Some(text);
                    }
                }
            }
        }
        if last_active_at.is_some() && summary.is_some() {
            break;
        }
    }

    let session_id = session_id.or_else(|| infer_session_id_from_filename(path));
    let session_id = session_id?;

    let title = thread_titles
        .get(&session_id)
        .map(|t| truncate_summary(t, TITLE_MAX_CHARS))
        .or_else(|| first_user_message.map(|t| truncate_summary(&t, TITLE_MAX_CHARS)))
        .or_else(|| {
            project_dir
                .as_deref()
                .and_then(path_basename)
                .map(|v| v.to_string())
        });

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
        resume_command: Some(format!("codex resume {session_id}")),
        forked_from_id,
    })
}

/// Check if a session_meta payload's `source` field contains a `subagent` key.
fn is_subagent_source(source: Option<&Value>) -> bool {
    source
        .and_then(|value| value.as_object())
        .map(|source| source.contains_key("subagent"))
        .unwrap_or(false)
}

/// Derive a title candidate from a user message, filtering out system injections.
fn title_candidate_from_user_message(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("# AGENTS.md")
        || trimmed.starts_with("<environment_context>")
    {
        return None;
    }

    if trimmed.starts_with(VSCODE_CONTEXT_PREFIX) {
        return extract_codex_prompt_from_ide_context(trimmed);
    }

    Some(trimmed.to_string())
}

/// Extract the actual user prompt from a VS Code IDE context block.
fn extract_codex_prompt_from_ide_context(text: &str) -> Option<String> {
    let normalized = text.replace("\r\n", "\n");
    let lines = normalized.lines().collect::<Vec<_>>();

    // VS Code injects the real prompt as the LAST "## My request for Codex:"
    // section, so keep the final matching heading. Earlier matches can be
    // headings that live inside the active selection / open file content.
    let mut prompt: Option<String> = None;
    for (index, line) in lines.iter().enumerate() {
        let Some(inline_prompt) = codex_request_heading_payload(line) else {
            continue;
        };

        if !inline_prompt.is_empty() {
            prompt = Some(inline_prompt.to_string());
            continue;
        }

        let following_prompt = lines[index + 1..].join("\n").trim().to_string();
        prompt = (!following_prompt.is_empty()).then_some(following_prompt);
    }

    prompt
}

/// Parse an inline payload from a "## My request for Codex:" heading line.
fn codex_request_heading_payload(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if !trimmed.starts_with('#') {
        return None;
    }

    let heading = trimmed.trim_start_matches('#').trim_start();
    let lowered = heading.to_ascii_lowercase();
    if !lowered.starts_with(CODEX_REQUEST_MARKER) {
        return None;
    }

    let suffix = heading[CODEX_REQUEST_MARKER.len()..].trim_start();
    if suffix.is_empty() {
        return Some("");
    }

    let Some(separator) = suffix.chars().next() else {
        return Some("");
    };
    if !matches!(separator, ':' | '：' | '-' | '—') {
        return None;
    }

    Some(
        suffix
            .trim_start_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '：' | '-' | '—'))
            .trim(),
    )
}

/// Fallback: extract a UUID-like session ID from the filename.
fn infer_session_id_from_filename(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_string_lossy();
    let s = file_name.as_ref();
    let len = s.len();

    for i in 0..len.saturating_sub(35) {
        let candidate = s.get(i..i + 36)?;
        let bytes = candidate.as_bytes();
        if bytes.len() == 36
            && bytes[8] == b'-'
            && bytes[13] == b'-'
            && bytes[18] == b'-'
            && bytes[23] == b'-'
            && bytes[..8].iter().all(|b| b.is_ascii_hexdigit())
            && bytes[9..13].iter().all(|b| b.is_ascii_hexdigit())
            && bytes[14..18].iter().all(|b| b.is_ascii_hexdigit())
            && bytes[19..23].iter().all(|b| b.is_ascii_hexdigit())
            && bytes[24..36].iter().all(|b| b.is_ascii_hexdigit())
        {
            return Some(candidate.to_string());
        }
    }
    None
}

// ─── Tool output parsing ─────────────────────────────────────────────────────

/// Split Codex tool output at the "Output:" delimiter.
///
/// Codex often embeds explanatory text before the actual output, separated
/// by "Output:" on its own line. We split so the explanatory text becomes
/// part of the message content and the structured output becomes `tool_result`.
///
/// Returns `(before_delimiter, after_delimiter)`.
fn split_codex_output(output: &str) -> (String, String) {
    let s = output.replace("\r\n", "\n");

    // Patterns ordered by specificity:
    // — "\nOutput:\n" / "\nOutput：\n"   "Output:" on its own line (full-width colon)
    // — "\nOutput: " / "\nOutput： "     "Output:" then inline content
    // — Start-of-string variants
    let patterns: [(&str, usize); 6] = [
        ("\nOutput:\n", 9),
        ("\nOutput：\n", 9),
        ("\nOutput: ", 8),
        ("\nOutput：", 8),
        ("Output:\n", 7),
        ("Output：\n", 7),
    ];

    for (pattern, skip) in &patterns {
        if let Some(pos) = s.find(pattern) {
            let before = s[..pos].trim().to_string();
            let after = s[pos + skip..].trim().to_string();
            return (before, after);
        }
    }

    // No delimiter found — entire text is the payload
    (String::new(), s)
}

// ─── Load messages ──────────────────────────────────────────────────────────

fn load_messages(path: &Path) -> Result<Vec<SessionMessage>, String> {
    let file = File::open(path).map_err(|e| format!("Failed to open session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut messages: Vec<SessionMessage> = Vec::new();
    // Track function_call message indices by call_id so parallel tool calls
    // each get their output merged into the correct message.
    let mut tool_call_map: HashMap<String, usize> = HashMap::new();

    for line in reader.lines() {
        let line = match line {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if value.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }

        let payload = match value.get("payload") {
            Some(payload) => payload,
            None => continue,
        };

        let payload_type = payload.get("type").and_then(Value::as_str).unwrap_or("");

        // Codex uses separate payload types for tool interactions
        let (role, content, tool_calls) = match payload_type {
            "message" => {
                let role = payload
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let content = payload.get("content").map(extract_text).unwrap_or_default();
                (role, content, None)
            }
            "function_call" => {
                let name = payload
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let arguments = payload
                    .get("arguments")
                    .and_then(Value::as_str)
                    .unwrap_or("{}")
                    .to_string();
                let truncated = if arguments.chars().count() > TOOL_CALL_INPUT_MAX_CHARS {
                    let mut s: String = arguments.chars().take(TOOL_CALL_INPUT_MAX_CHARS).collect();
                    s.push_str("...");
                    s
                } else {
                    arguments
                };
                let call_id = payload
                    .get("call_id")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string());
                let tool_calls = Some(vec![ToolCallInfo {
                    name: name.to_string(),
                    input: truncated,
                    call_id: call_id.clone(),
                }]);
                // Track this call by its call_id so the matching output
                // can be merged into this message (supports parallel calls).
                if let Some(ref id) = call_id {
                    tool_call_map.insert(id.clone(), messages.len());
                }
                (
                    "assistant".to_string(),
                    format!("[Tool: {name}]"),
                    tool_calls,
                )
            }
            "function_call_output" => {
                let output = payload
                    .get("output")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let call_id = payload.get("call_id").and_then(Value::as_str);

                // Merge into the matching function_call's message by call_id
                // (handles parallel tool calls correctly).
                if let Some(cid) = call_id.and_then(|id| tool_call_map.remove(id)) {
                    if let Some(msg) = messages.get_mut(cid) {
                        if msg.tool_calls.is_some() {
                            let (explanatory, result) = split_codex_output(&output);
                            if !result.is_empty() {
                                msg.tool_result = Some(ToolResultInfo {
                                    content: result,
                                    call_id: None,
                                });
                            }
                            if !explanatory.is_empty() {
                                if !msg.content.is_empty() && !msg.content.ends_with('\n') {
                                    msg.content.push('\n');
                                }
                                msg.content.push_str(&explanatory);
                            }
                            // Consumed — skip creating a separate message
                            continue;
                        }
                    }
                }

                // Fallback: no matching function_call to merge with
                ("tool".to_string(), output, None)
            }
            _ => continue,
        };

        if content.trim().is_empty() {
            continue;
        }

        let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);

        messages.push(SessionMessage {
            role,
            content,
            ts,
            usage: None,
            cumulative_usage: None,
            tool_calls,
            tool_result: None,
        });
    }

    Ok(messages)
}

// ─── Move session ───────────────────────────────────────────────────────────

/// Move a Codex session file (JSONL only, no sidecar) to a destination directory.
fn move_session(source_path: &Path, dest_dir: &Path) -> Result<(), String> {
    move_single_file(source_path, dest_dir)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TEST_ENV_LOCK;
    use tempfile::tempdir;

    // Use the global shared lock to prevent parallel tests from racing on env vars.
    static ENV_LOCK: &std::sync::Mutex<()> = &TEST_ENV_LOCK;

    fn write_codex_session(path: &Path, session_id: &str, message: &str) {
        std::fs::write(
            path,
            format!(
                "{{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\",\"cwd\":\"/tmp/project\"}}}}\n\
                 {{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":\"{message}\"}}}}\n",
            ),
        )
            .expect("write session");
    }

    fn write_session_index(codex_dir: &Path, entries: &[(&str, &str)]) {
        let index_path = codex_dir.join(CODEX_SESSION_INDEX_FILENAME);
        let mut content = String::new();
        for (id, name) in entries {
            content.push_str(&format!("{{\"id\":\"{id}\",\"thread_name\":\"{name}\"}}\n"));
        }
        std::fs::write(&index_path, content).expect("write session index");
    }

    fn setup_test_env() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
        let guard = ENV_LOCK.lock().expect("lock");
        let temp = tempdir().expect("tempdir");
        std::env::set_var("SESSION_MANAGER_TEST_HOME", temp.path());
        (temp, guard)
    }

    #[test]
    fn codex_provider_trait_impl() {
        let provider = CodexProvider;
        assert_eq!(provider.id(), "codex");
        assert_eq!(provider.roots().len(), 2);
    }

    #[test]
    fn scan_sessions_includes_active_and_archived() {
        let (_temp, _guard) = setup_test_env();

        let provider = CodexProvider;
        let active = crate::config::get_codex_sessions_dir();
        let archived = crate::config::get_codex_archive_dir();
        std::fs::create_dir_all(&active).expect("active dir");
        std::fs::create_dir_all(&archived).expect("archived dir");

        write_codex_session(&active.join("active.jsonl"), "active-id", "Active session");
        write_codex_session(
            &archived.join("archived.jsonl"),
            "archived-id",
            "Archived session",
        );

        let active_sessions = provider.scan_sessions(&active);
        let archived_sessions = provider.scan_sessions(&archived);
        let ids: Vec<&str> = active_sessions
            .iter()
            .chain(archived_sessions.iter())
            .map(|s| s.session_id.as_str())
            .collect();

        assert!(ids.contains(&"active-id"));
        assert!(ids.contains(&"archived-id"));
    }

    #[test]
    fn parse_session_uses_first_user_message_as_title() {
        let (_temp, _guard) = setup_test_env();

        let codex_dir = crate::config::get_codex_dir();
        std::fs::create_dir_all(&codex_dir).expect("codex dir");

        let path = codex_dir.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp/project\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"How do I deploy?\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:14Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"Here is how...\"}}\n"
            ),
        )
            .expect("write");

        let provider = CodexProvider;
        let meta = provider.parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("How do I deploy?"));
    }

    #[test]
    fn parse_session_prefers_thread_title_from_session_index() {
        let (_temp, _guard) = setup_test_env();

        let codex_dir = crate::config::get_codex_dir();
        std::fs::create_dir_all(&codex_dir).expect("codex dir");

        let path = codex_dir.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp/project\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"How do I deploy?\"}}\n"
            ),
        )
            .expect("write");

        write_session_index(&codex_dir, &[("test-id", "Renamed deployment thread")]);

        let provider = CodexProvider;
        let meta = provider.parse_session(&path).unwrap();
        assert_eq!(meta.title.as_deref(), Some("Renamed deployment thread"));
    }

    #[test]
    fn parse_session_falls_back_to_dir_basename() {
        let (_temp, _guard) = setup_test_env();

        let codex_dir = crate::config::get_codex_dir();
        std::fs::create_dir_all(&codex_dir).expect("codex dir");

        let path = codex_dir.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp/my-project\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"Hello\"}}\n"
            ),
        )
            .expect("write");

        let provider = CodexProvider;
        let meta = provider.parse_session(&path).unwrap();
        // No user message -> falls back to dir basename
        assert_eq!(meta.title.as_deref(), Some("my-project"));
    }

    #[test]
    fn parse_session_skips_subagent_sessions() {
        let (_temp, _guard) = setup_test_env();

        let codex_dir = crate::config::get_codex_dir();
        std::fs::create_dir_all(&codex_dir).expect("codex dir");

        let path = codex_dir.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-04-28T10:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"subagent-id\",\"cwd\":\"/tmp/project\",\"originator\":\"codex-tui\",\"source\":{\"subagent\":{\"thread_spawn\":{\"parent_thread_id\":\"parent-id\",\"depth\":1,\"agent_role\":\"explorer\"}}}}}\n",
                "{\"timestamp\":\"2026-04-28T10:00:01Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"Inspect the project\"}}\n"
            ),
        )
            .expect("write");

        let provider = CodexProvider;
        assert!(provider.parse_session(&path).is_none());
    }

    #[test]
    fn parse_session_skips_agents_md_injection() {
        let (_temp, _guard) = setup_test_env();

        let codex_dir = crate::config::get_codex_dir();
        std::fs::create_dir_all(&codex_dir).expect("codex dir");

        let path = codex_dir.join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp/project\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",\"content\":\"<permissions>\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"# AGENTS.md instructions for /tmp/project\\n<INSTRUCTIONS>Do stuff</INSTRUCTIONS>\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:14Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"Fix the login bug\"}}\n"
            ),
        )
            .expect("write");

        let provider = CodexProvider;
        let meta = provider.parse_session(&path).unwrap();
        // Should skip AGENTS.md injection and use the real user message
        assert_eq!(meta.title.as_deref(), Some("Fix the login bug"));
    }

    #[test]
    fn load_messages_includes_function_call_and_output() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"list files\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:14Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"shell\",\"arguments\":\"{\\\"cmd\\\":[\\\"ls\\\"]}\",\"call_id\":\"call_1\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:15Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_1\",\"output\":\"file1.txt\\nfile2.txt\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:16Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Done.\"}]}}\n",
            ),
        )
            .expect("write");

        let msgs = load_messages(&path).expect("load");
        // function_call_output is now merged into the function_call message
        assert_eq!(msgs.len(), 3);

        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "list files");

        assert_eq!(msgs[1].role, "assistant");
        assert!(msgs[1].content.contains("[Tool: shell]"));
        assert!(msgs[1].tool_calls.is_some());
        assert!(msgs[1].tool_result.is_some());
        assert_eq!(
            msgs[1].tool_result.as_ref().unwrap().content,
            "file1.txt\nfile2.txt"
        );

        assert_eq!(msgs[2].role, "assistant");
        assert_eq!(msgs[2].content, "Done.");
    }

    #[test]
    fn load_messages_parses_roles() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"hello\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:14Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"read_file\",\"arguments\":\"{}\",\"call_id\":\"c1\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:15Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"c1\",\"output\":\"file content\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:16Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"response text\"}}\n",
            ),
        )
            .expect("write");

        let msgs = load_messages(&path).expect("load");
        // function_call_output is now merged into the function_call message
        assert_eq!(msgs.len(), 3);

        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
        assert_eq!(msgs[1].role, "assistant");
        assert!(msgs[1].content.contains("[Tool: read_file]"));
        assert!(msgs[1].tool_calls.is_some());
        assert!(msgs[1].tool_result.is_some());
        assert_eq!(
            msgs[1].tool_result.as_ref().unwrap().content,
            "file content"
        );
        assert_eq!(msgs[2].role, "assistant");
        assert_eq!(msgs[2].content, "response text");
    }

    #[test]
    fn validate_session_id_ok() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"hi\"}}\n"
            ),
        )
            .expect("write");

        let provider = CodexProvider;

        // validate_session_id uses parse_session which needs session_index.jsonl
        // to exist or at least not error — just ensure codex dir exists
        let codex_dir = crate::config::get_codex_dir();
        let _ = std::fs::create_dir_all(&codex_dir);

        assert!(provider.validate_session_id(&path, "test-id").is_ok());
    }

    #[test]
    fn validate_session_id_mismatch() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"hi\"}}\n"
            ),
        )
            .expect("write");

        let provider = CodexProvider;

        let codex_dir = crate::config::get_codex_dir();
        let _ = std::fs::create_dir_all(&codex_dir);

        assert!(provider.validate_session_id(&path, "wrong-id").is_err());
    }

    #[test]
    fn move_session_moves_file() {
        let temp = tempdir().expect("tempdir");
        let source_dir = temp.path().join("source");
        let dest_dir = temp.path().join("dest");
        std::fs::create_dir_all(&source_dir).expect("source dir");

        let source_path = source_dir.join("session.jsonl");
        std::fs::write(
            &source_path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"hi\"}}\n"
            ),
        )
            .expect("write");

        move_session(&source_path, &dest_dir).expect("move");

        assert!(!source_path.exists(), "source file should be gone");
        assert!(
            dest_dir.join("session.jsonl").exists(),
            "dest file should exist"
        );
    }

    #[test]
    fn codex_user_events_extracts_user_messages() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"hello\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:14Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"world\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:15Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"second message\"}}\n",
            ),
        )
            .expect("write");

        let provider = CodexProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], "hello");
        assert_eq!(events[1], "second message");
    }

    #[test]
    fn codex_user_events_skips_function_call_and_output() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"list files\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:14Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"shell\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:15Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_1\"}}\n",
            ),
        )
            .expect("write");

        let provider = CodexProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], "list files");
    }

    #[test]
    fn codex_user_events_skips_non_user_roles() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"test-id\",\"cwd\":\"/tmp\"}}\n",
                "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"response\"}}\n",
            ),
        )
            .expect("write");

        let provider = CodexProvider;
        let events = provider.user_events(&path).expect("user_events");
        assert_eq!(events.len(), 0);
    }
}
