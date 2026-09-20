use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::fs_utils;
use crate::session_manager::types::ToolResultInfo;
use crate::session_manager::{SessionLocator, SessionMessage, SessionMeta, ToolCallInfo};

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
        let files = fs_utils::walk_jsonl_paths(root);

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

// ─── User events for fork tree ──────────────────────────────────────────────

/// Extract user input text events from a Codex session file.
/// Returns all user message texts in chronological order.
fn user_events_from_path(path: &Path) -> Result<Vec<String>, String> {
    let (events, mode, saw_item_user, saw_response_user) = collect_user_events(path, false)?;
    // A file labelled paginated that carries no item_completed user messages
    // would read as an empty conversation — fall back to the legacy channel.
    if mode == HistoryMode::Paginated && !saw_item_user && saw_response_user {
        return Ok(collect_user_events(path, true)?.0);
    }
    Ok(events)
}

fn collect_user_events(
    path: &Path,
    force_legacy: bool,
) -> Result<(Vec<String>, HistoryMode, bool, bool), String> {
    let file = File::open(path).map_err(|e| format!("Failed to open session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut events: Vec<String> = Vec::new();
    let mut mode = HistoryMode::Legacy;
    let mut mode_resolved = false;
    let mut saw_item_user = false;
    let mut saw_response_user = false;

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

        if !mode_resolved {
            mode = if force_legacy {
                HistoryMode::Legacy
            } else {
                history_mode_from_record(&value)
            };
            mode_resolved = true;
        }

        if mode == HistoryMode::Paginated {
            if let Some((role, text)) = item_completed_message(&value) {
                if role == "user" {
                    saw_item_user = true;
                    if !text.trim().is_empty() {
                        events.push(text.trim().to_string());
                    }
                }
                continue;
            }
        }

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

        if mode == HistoryMode::Paginated {
            // The response_item twin duplicates every item_completed message
            // (and adds injected context under the user role); skipping it
            // keeps the event list double-count free.
            saw_response_user = true;
            continue;
        }

        let text = payload.get("content").map(extract_text).unwrap_or_default();
        if !text.trim().is_empty() {
            events.push(text.trim().to_string());
        }
    }

    Ok((events, mode, saw_item_user, saw_response_user))
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
    let mut item_user_message: Option<String> = None;
    let mut forked_from_id: Option<String> = None;
    let mut mode = HistoryMode::Legacy;

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
            mode = history_mode_from_record(&value);
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
        // First user message via the paginated item_completed channel — this
        // channel carries only real user input, no injected context.
        if mode == HistoryMode::Paginated && item_user_message.is_none() {
            if let Some((role, text)) = item_completed_message(&value) {
                if role == "user" {
                    if let Some(title) = title_candidate_from_user_message(&text) {
                        item_user_message = Some(title);
                    }
                }
            }
        }
        // Extract first user message as title candidate from the legacy
        // response_item channel (sole source for legacy files; fallback for
        // paginated files whose head window holds no item_completed user
        // message yet).
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
        let title_found = if mode == HistoryMode::Paginated {
            item_user_message.is_some()
        } else {
            first_user_message.is_some()
        };
        if session_id.is_some() && project_dir.is_some() && created_at.is_some() && title_found {
            break;
        }
    }

    // Extract last_active_at and summary from tail lines (reverse order)
    let mut last_active_at: Option<i64> = None;
    let mut summary: Option<String> = None;
    let mut item_summary: Option<String> = None;

    for line in tail.iter().rev() {
        let value: Value = match serde_json::from_str(line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };
        if last_active_at.is_none() {
            last_active_at = value.get("timestamp").and_then(parse_timestamp_to_ms);
        }
        if mode == HistoryMode::Paginated && item_summary.is_none() {
            if let Some((_, text)) = item_completed_message(&value) {
                if !text.trim().is_empty() {
                    item_summary = Some(text);
                }
            }
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
        let summary_found = if mode == HistoryMode::Paginated {
            item_summary.is_some()
        } else {
            summary.is_some()
        };
        if last_active_at.is_some() && summary_found {
            break;
        }
    }

    let session_id = session_id.or_else(|| infer_session_id_from_filename(path));
    let session_id = session_id?;

    let title_candidate = if mode == HistoryMode::Paginated {
        item_user_message.or(first_user_message)
    } else {
        first_user_message
    };
    let title = thread_titles
        .get(&session_id)
        .map(|t| truncate_summary(t, TITLE_MAX_CHARS))
        .or_else(|| title_candidate.map(|t| truncate_summary(&t, TITLE_MAX_CHARS)))
        .or_else(|| {
            project_dir
                .as_deref()
                .and_then(path_basename)
                .map(|v| v.to_string())
        });

    let summary = if mode == HistoryMode::Paginated {
        item_summary.or(summary)
    } else {
        summary
    };

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
        locator: Some(SessionLocator::File {
            path: path.to_string_lossy().to_string(),
        }),
        resume_command: Some(format!("codex resume {session_id}")),
        forked_from_id,
    })
}

// ─── History mode (paginated rollout generation) ─────────────────────────────

/// Message-channel generation of a Codex rollout file.
///
/// Codex 0.153+ writes threads in "paginated" mode: user/assistant messages
/// surface through `event_msg`/`item_completed` records embedding a TurnItem,
/// while `response_item` records keep carrying a model-facing duplicate of
/// every message plus injected context (AGENTS.md instructions, app context)
/// under user/developer roles. Parsing both channels per file would double
/// every message, so the generation decides which channel to trust:
///
/// - The first `session_meta` record's `payload.history_mode` is authoritative
///   (`"paginated"` ⇒ new generation). The field is absent in older files —
///   the writer side defaults it to `"legacy"` — so absence means legacy.
/// - The line-level `ordinal` field (present on every record of a paginated
///   rollout, including the leading meta line) acts as a secondary
///   confirmation: a file claiming paginated without it is parsed as legacy.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HistoryMode {
    Legacy,
    Paginated,
}

fn history_mode_from_record(value: &Value) -> HistoryMode {
    if value.get("type").and_then(Value::as_str) != Some("session_meta") {
        return HistoryMode::Legacy;
    }
    let paginated = value
        .pointer("/payload/history_mode")
        .and_then(Value::as_str)
        == Some("paginated");
    if paginated && value.get("ordinal").is_some() {
        HistoryMode::Paginated
    } else {
        HistoryMode::Legacy
    }
}

/// Extract the user/assistant message carried by an `event_msg`/`item_completed`
/// record (the paginated message channel). Returns `(role, text)`; tool /
/// reasoning TurnItems return `None` — tool calls keep flowing through their
/// `response_item` records in both generations.
///
/// Unlike `response_item` user-role records, this channel never contains
/// injected context, so it needs no AGENTS.md / environment filtering.
fn item_completed_message(value: &Value) -> Option<(&'static str, String)> {
    let payload = value.get("payload")?;
    if payload.get("type").and_then(Value::as_str) != Some("item_completed") {
        return None;
    }
    let item = payload.get("item")?;
    let role = match item.get("type").and_then(Value::as_str) {
        Some("UserMessage") => "user",
        Some("AgentMessage") => "assistant",
        _ => return None,
    };
    let text = item.get("content").map(extract_text).unwrap_or_default();
    Some((role, text))
}

/// Whether a `response_item` message is injected context scaffolding rather
/// than conversation content, i.e. whether it has no item_completed twin and
/// must be surfaced from the `response_item` ledger (the record of what the
/// model actually received) to stay visible.
///
/// Classification order:
/// 1. developer/system roles are always scaffolding.
/// 2. `content_item_kinds` (machine-readable, stamped by the codex writer)
///    decides for user messages when present: real user input is kind
///    `user.text`, everything else (agents_md.instructions,
///    environment_context, turn_aborted, …) is injected context. Unknown
///    future kinds classify as injected — the failure direction is one extra
///    displayed message, never a dropped one.
/// 3. Records without kinds (older writers) fall back to codex's contextual
///    fragment markers; start AND end marker must both match, mirroring codex
///    `matches_marked_text`, so a real user message that merely opens with a
///    marker is never mistaken for scaffolding (that would double-count it
///    against its item_completed twin).
fn is_contextual_scaffolding(role: &str, text: &str, kinds: Option<&[String]>) -> bool {
    match role {
        "developer" | "system" => true,
        "user" => match kinds {
            Some(kinds) if !kinds.is_empty() => !kinds.iter().any(|kind| kind == "user.text"),
            _ => CONTEXTUAL_USER_MARKERS.iter().any(|(start, end)| {
                let opened = text
                    .trim_start()
                    .get(..start.len())
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(start));
                let closed = text
                    .trim_end()
                    .len()
                    .checked_sub(end.len())
                    .and_then(|at| text.trim_end().get(at..))
                    .is_some_and(|candidate| candidate.eq_ignore_ascii_case(end));
                opened && closed
            }),
        },
        _ => false,
    }
}

/// Contextual user fragment (start, end) markers, mirroring codex
/// `contextual_user_message.rs` and its fragment types. Only consulted for
/// records written before the writer stamped `content_item_kinds`; keep it a
/// subset of codex's contextual filter.
const CONTEXTUAL_USER_MARKERS: &[(&str, &str)] = &[
    ("# AGENTS.md instructions", "</INSTRUCTIONS>"),
    ("<environment_context>", "</environment_context>"),
    ("<user_shell_command>", "</user_shell_command>"),
    ("<turn_aborted>", "</turn_aborted>"),
    ("<subagent_notification>", "</subagent_notification>"),
    ("<codex_internal_context", "</codex_internal_context>"),
    ("<goal_context>", "</goal_context>"),
    ("<skill>", "</skill>"),
];

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
        || trimmed.starts_with("<turn_aborted>")
        || trimmed.starts_with("<subagent_notification>")
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
/// Codex tool output often carries a metadata header (e.g. "Chunk ID /
/// Wall time / Process exited" for exec commands) before the actual output,
/// separated by "Output:" on its own line. We split so both halves stay in
/// the structured `tool_result` (header + payload); neither half is
/// assistant commentary.
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
    let (messages, mode, saw_item_message, saw_response_message) = collect_messages(path, false)?;
    // A file labelled paginated that carries no item_completed messages would
    // read as an empty conversation — fall back to the legacy channel.
    if mode == HistoryMode::Paginated && !saw_item_message && saw_response_message {
        return Ok(collect_messages(path, true)?.0);
    }
    Ok(messages)
}

fn collect_messages(
    path: &Path,
    force_legacy: bool,
) -> Result<(Vec<SessionMessage>, HistoryMode, bool, bool), String> {
    let file = File::open(path).map_err(|e| format!("Failed to open session file: {e}"))?;
    let reader = BufReader::new(file);
    let mut messages: Vec<SessionMessage> = Vec::new();
    // Track function_call message indices by call_id so parallel tool calls
    // each get their output merged into the correct message.
    let mut tool_call_map: HashMap<String, usize> = HashMap::new();
    let mut mode = HistoryMode::Legacy;
    let mut mode_resolved = false;
    let mut saw_item_message = false;
    let mut saw_response_message = false;

    for line in reader.lines() {
        let line = match line {
            Ok(value) => value,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&line) {
            Ok(parsed) => parsed,
            Err(_) => continue,
        };

        if !mode_resolved {
            mode = if force_legacy {
                HistoryMode::Legacy
            } else {
                history_mode_from_record(&value)
            };
            mode_resolved = true;
        }

        // Paginated generation: messages come from item_completed records.
        if mode == HistoryMode::Paginated {
            if let Some((role, text)) = item_completed_message(&value) {
                if !text.trim().is_empty() {
                    saw_item_message = true;
                    let ts = value.get("timestamp").and_then(parse_timestamp_to_ms);
                    messages.push(SessionMessage {
                        role: role.to_string(),
                        content: text,
                        ts,
                        usage: None,
                        cumulative_usage: None,
                        tool_calls: None,
                        tool_result: None,
                    });
                }
                continue;
            }
        }

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
                if mode == HistoryMode::Paginated {
                    saw_response_message = true;
                    // response_item is the ledger of what the model actually
                    // received; item_completed is codex's lossy UI projection
                    // of it. Conversation user/assistant messages have
                    // item_completed twins — skip them to avoid double
                    // counting — but injected context never reaches the item
                    // channel, so pass it through here (rendered as system
                    // blocks by the frontend, same as legacy sessions).
                    let kinds = payload
                        .pointer("/internal_chat_message_metadata_passthrough/content_item_kinds")
                        .and_then(Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect::<Vec<String>>()
                        });
                    if !is_contextual_scaffolding(&role, &content, kinds.as_deref())
                        || content.trim().is_empty()
                    {
                        continue;
                    }
                }
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
                            // Both halves are machine tool output (the
                            // pre-"Output:" header is metadata like "Chunk
                            // ID/Wall time", not assistant commentary) — keep
                            // them together in tool_result. Appending the
                            // header to `content` polluted both the message
                            // view and the Q&A export's joined answers.
                            let merged = match (explanatory.is_empty(), result.is_empty()) {
                                (true, false) => result,
                                (false, true) => explanatory,
                                (false, false) => format!("{explanatory}\n\n{result}"),
                                (true, true) => String::new(),
                            };
                            if !merged.is_empty() {
                                msg.tool_result = Some(ToolResultInfo {
                                    content: merged,
                                    call_id: None,
                                });
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

    Ok((messages, mode, saw_item_message, saw_response_message))
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
    fn scan_sessions_skips_directory_link_cycle() {
        let (_temp, _guard) = setup_test_env();

        let root = crate::config::get_codex_sessions_dir();
        let real = root.join("real");
        std::fs::create_dir_all(&real).expect("real dir");
        write_codex_session(&real.join("session.jsonl"), "cycle-id", "Cycle session");

        let link = real.join("loop");
        create_dir_link(&real, &link).expect("create directory link");

        let provider = CodexProvider;
        let sessions = provider.scan_sessions(&root);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "cycle-id");
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

    // ─── Paginated history (item_completed channel) ──────────────────────────

    const PAGINATED_META: &str = "{\"timestamp\":\"2026-09-19T06:06:49.051Z\",\"ordinal\":0,\"type\":\"session_meta\",\"payload\":{\"id\":\"pag-id\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-09-19T06:06:45.843Z\",\"history_mode\":\"paginated\"}}";

    const PAGINATED_META_WITHOUT_ORDINAL: &str = "{\"timestamp\":\"2026-09-19T06:06:49.051Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"pag-id\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-09-19T06:06:45.843Z\",\"history_mode\":\"paginated\"}}";

    const LEGACY_META: &str = "{\"timestamp\":\"2026-03-06T21:50:12Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"pag-id\",\"cwd\":\"/tmp/project\"}}";

    // A paginated turn as the writer really lays it down: conversation
    // messages carry an item_completed twin (UI channel) and stamped
    // `content_item_kinds`; injected context exists only as response_item
    // records with non-user kinds and never reaches the item channel. The
    // last user message opens with the AGENTS.md marker but is real input
    // (user.text kind + item twin) — a marker-prefix false positive would
    // show it twice.
    const PAGINATED_DUAL_CHANNEL_BODY: &str = concat!(
        "{\"ordinal\":1,\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",\"content\":[{\"type\":\"input_text\",\"text\":\"<app-context> desktop context\"}],\"internal_chat_message_metadata_passthrough\":{\"content_item_kinds\":[\"apps.instructions\",\"generic.developer_instructions\"]}}}\n",
        "{\"ordinal\":2,\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"# AGENTS.md instructions for /tmp/project\\n\\n<INSTRUCTIONS>\\nbe nice\\n</INSTRUCTIONS>\"}],\"internal_chat_message_metadata_passthrough\":{\"content_item_kinds\":[\"agents_md.instructions\"]}}}\n",
        "{\"ordinal\":3,\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"<turn_aborted>\\nThe user interrupted the previous turn on purpose.\\n</turn_aborted>\"}],\"internal_chat_message_metadata_passthrough\":{\"content_item_kinds\":[\"generic.turn_aborted\"]}}}\n",
        "{\"ordinal\":4,\"timestamp\":\"2026-09-19T06:06:50.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"How do I deploy?\"}],\"internal_chat_message_metadata_passthrough\":{\"content_item_kinds\":[\"user.text\"]}}}\n",
        "{\"ordinal\":5,\"timestamp\":\"2026-09-19T06:06:50.100Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"thread_id\":\"pag-id\",\"turn_id\":\"turn-1\",\"item\":{\"type\":\"UserMessage\",\"id\":\"u1\",\"client_id\":\"c1\",\"content\":[{\"type\":\"text\",\"text\":\"How do I deploy?\",\"text_elements\":[]}]},\"started_at_ms\":1789798010000,\"completed_at_ms\":1789798010000}}\n",
        "{\"ordinal\":6,\"timestamp\":\"2026-09-19T06:06:51.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"shell\",\"arguments\":\"{\\\"cmd\\\":\\\"ls\\\"}\",\"call_id\":\"call_1\"}}\n",
        "{\"ordinal\":7,\"timestamp\":\"2026-09-19T06:06:52.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_1\",\"output\":\"Chunk ID: 1\\nOutput:\\nok\"}}\n",
        "{\"ordinal\":8,\"timestamp\":\"2026-09-19T06:06:53.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Deployed.\"}]}}\n",
        "{\"ordinal\":9,\"timestamp\":\"2026-09-19T06:06:54.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"thread_id\":\"pag-id\",\"turn_id\":\"turn-1\",\"item\":{\"type\":\"AgentMessage\",\"id\":\"msg_1\",\"content\":[{\"type\":\"Text\",\"text\":\"Deployed.\"}],\"phase\":\"commentary\"},\"started_at_ms\":1789798013000,\"completed_at_ms\":1789798014000}}\n",
        "{\"ordinal\":10,\"timestamp\":\"2026-09-19T06:06:55.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"# AGENTS.md instructions say be nice\"}],\"internal_chat_message_metadata_passthrough\":{\"content_item_kinds\":[\"user.text\"]}}}\n",
        "{\"ordinal\":11,\"timestamp\":\"2026-09-19T06:06:55.100Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"thread_id\":\"pag-id\",\"turn_id\":\"turn-1\",\"item\":{\"type\":\"UserMessage\",\"id\":\"u2\",\"client_id\":\"c2\",\"content\":[{\"type\":\"text\",\"text\":\"# AGENTS.md instructions say be nice\",\"text_elements\":[]}]},\"started_at_ms\":1789798015000,\"completed_at_ms\":1789798015000}}\n",
    );

    // The same turn without the item_completed twins and ordinals — a legacy
    // rollout.
    const LEGACY_RESPONSE_ITEM_BODY: &str = concat!(
        "{\"timestamp\":\"2026-03-06T21:50:13Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"developer\",\"content\":[{\"type\":\"input_text\",\"text\":\"<app-context> desktop context\"}]}}\n",
        "{\"timestamp\":\"2026-03-06T21:50:14Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"# AGENTS.md instructions for /tmp/project\"}]}}\n",
        "{\"timestamp\":\"2026-03-06T21:50:15Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"How do I deploy?\"}]}}\n",
        "{\"timestamp\":\"2026-03-06T21:50:16Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"shell\",\"arguments\":\"{\\\"cmd\\\":\\\"ls\\\"}\",\"call_id\":\"call_1\"}}\n",
        "{\"timestamp\":\"2026-03-06T21:50:17Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call_1\",\"output\":\"Chunk ID: 1\\nOutput:\\nok\"}}\n",
        "{\"timestamp\":\"2026-03-06T21:50:18Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"Deployed.\"}]}}\n",
    );

    fn message_outline(messages: &[SessionMessage]) -> Vec<(&str, &str)> {
        messages
            .iter()
            .map(|m| (m.role.as_str(), m.content.as_str()))
            .collect()
    }

    #[test]
    fn history_mode_detection_variants() {
        let meta = |extras: &str| -> Value { serde_json::from_str(extras).expect("valid json") };

        assert_eq!(
            history_mode_from_record(&meta(PAGINATED_META)),
            HistoryMode::Paginated
        );
        // Secondary confirmation: no ordinal on the meta line ⇒ legacy.
        assert_eq!(
            history_mode_from_record(&meta(PAGINATED_META_WITHOUT_ORDINAL)),
            HistoryMode::Legacy
        );
        // Field absent ⇒ legacy (the writer's serde default).
        assert_eq!(
            history_mode_from_record(&meta(LEGACY_META)),
            HistoryMode::Legacy
        );
        // Non-meta records never carry generation info.
        assert_eq!(
            history_mode_from_record(&meta(
                "{\"ordinal\":1,\"type\":\"response_item\",\"payload\":{\"type\":\"message\"}}"
            )),
            HistoryMode::Legacy
        );
    }

    #[test]
    fn load_messages_paginated_uses_item_completed_channel() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            format!("{PAGINATED_META}\n{PAGINATED_DUAL_CHANNEL_BODY}"),
        )
        .expect("write");

        let messages = load_messages(&path).expect("load_messages");
        assert_eq!(
            message_outline(&messages),
            vec![
                ("developer", "<app-context> desktop context"),
                (
                    "user",
                    "# AGENTS.md instructions for /tmp/project\n\n<INSTRUCTIONS>\nbe nice\n</INSTRUCTIONS>"
                ),
                (
                    "user",
                    "<turn_aborted>\nThe user interrupted the previous turn on purpose.\n</turn_aborted>"
                ),
                ("user", "How do I deploy?"),
                ("assistant", "[Tool: shell]"),
                ("assistant", "Deployed."),
                ("user", "# AGENTS.md instructions say be nice"),
            ]
        );
        // Tool calls stay on the response_item channel and still merge output.
        assert_eq!(
            messages[4].tool_result.as_ref().map(|r| r.content.as_str()),
            Some("Chunk ID: 1\n\nok")
        );
        // Double-count tripwire: every conversation twin exists on both
        // channels, but each text must surface exactly once.
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.content.contains("How do I deploy?"))
                .count(),
            1
        );
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.content.contains("Deployed."))
                .count(),
            1
        );
        // A real user message that merely opens with a scaffolding marker must
        // come from its item_completed twin exactly once, not twice.
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.content.contains("say be nice"))
                .count(),
            1
        );
        // Injected context is scaffolding the model actually received: it
        // never reaches the item_completed channel, so it must surface exactly
        // once from the response_item ledger, in place.
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.content.contains("app-context"))
                .count(),
            1
        );
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.content.contains("AGENTS.md instructions for"))
                .count(),
            1
        );
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.content.contains("turn_aborted"))
                .count(),
            1
        );
    }

    #[test]
    fn load_messages_paginated_only_injections_falls_back_to_legacy() {
        // A paginated-labelled file whose only response_item messages are
        // injected context carries no item channel at all: the legacy
        // fallback must take over (it renders everything, including the
        // injections).
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            format!("{PAGINATED_META}\n{LEGACY_RESPONSE_ITEM_BODY}"),
        )
        .expect("write");

        let messages = load_messages(&path).expect("load_messages");
        assert_eq!(
            message_outline(&messages),
            vec![
                ("developer", "<app-context> desktop context"),
                ("user", "# AGENTS.md instructions for /tmp/project"),
                ("user", "How do I deploy?"),
                ("assistant", "[Tool: shell]"),
                ("assistant", "Deployed."),
            ]
        );
    }

    #[test]
    fn contextual_scaffolding_detection() {
        // Roles without an item_completed twin are always scaffolding.
        assert!(is_contextual_scaffolding("developer", "<app-context> x", None));
        assert!(is_contextual_scaffolding("system", "anything", None));
        // Kind metadata decides for user messages: real input is user.text.
        assert!(!is_contextual_scaffolding(
            "user",
            "How do I deploy?",
            Some(&["user.text".to_string()])
        ));
        assert!(is_contextual_scaffolding(
            "user",
            "# AGENTS.md instructions for /tmp",
            Some(&["agents_md.instructions".to_string()])
        ));
        // Unknown future kinds fail toward showing, never dropping.
        assert!(is_contextual_scaffolding(
            "user",
            "whatever",
            Some(&["gizmo.instructions".to_string()])
        ));
        // Marker fallback for kinds-less records: start AND end marker must
        // both match, mirroring codex matches_marked_text.
        assert!(is_contextual_scaffolding(
            "user",
            "# AGENTS.md instructions for /tmp\n\n<INSTRUCTIONS>\nbe nice\n</INSTRUCTIONS>",
            None
        ));
        assert!(is_contextual_scaffolding(
            "user",
            "<turn_aborted>\nThe user interrupted the previous turn on purpose.\n</turn_aborted>",
            None
        ));
        // A real user message that merely opens with a scaffolding marker is
        // conversation content (it has an item twin) — classifying it as
        // scaffolding would double-count it.
        assert!(!is_contextual_scaffolding(
            "user",
            "# AGENTS.md instructions say be nice",
            None
        ));
        assert!(!is_contextual_scaffolding("assistant", "Deployed.", None));
    }

    #[test]
    fn load_messages_legacy_channel_unchanged() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(&path, format!("{LEGACY_META}\n{LEGACY_RESPONSE_ITEM_BODY}"))
            .expect("write");

        // Legacy files keep the exact response_item behavior, injected
        // context included.
        let messages = load_messages(&path).expect("load_messages");
        assert_eq!(
            message_outline(&messages),
            vec![
                ("developer", "<app-context> desktop context"),
                ("user", "# AGENTS.md instructions for /tmp/project"),
                ("user", "How do I deploy?"),
                ("assistant", "[Tool: shell]"),
                ("assistant", "Deployed."),
            ]
        );
    }

    #[test]
    fn load_messages_paginated_label_without_ordinal_parses_as_legacy() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            format!("{PAGINATED_META_WITHOUT_ORDINAL}\n{PAGINATED_DUAL_CHANNEL_BODY}"),
        )
        .expect("write");

        // history_mode claims paginated but the meta line carries no ordinal:
        // the secondary confirmation fails, the legacy channel stays
        // authoritative, and item_completed lines never leak in.
        let messages = load_messages(&path).expect("load_messages");
        assert_eq!(
            messages
                .iter()
                .filter(|m| m.content == "How do I deploy?")
                .count(),
            1
        );
        assert!(messages.iter().any(|m| m.role == "developer"));
    }

    #[test]
    fn load_messages_paginated_without_item_channel_falls_back() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            format!("{PAGINATED_META}\n{LEGACY_RESPONSE_ITEM_BODY}"),
        )
        .expect("write");

        // Defensive fallback: a paginated-labelled file with no item_completed
        // messages must not read as an empty conversation.
        let messages = load_messages(&path).expect("load_messages");
        assert_eq!(messages.len(), 5);
        assert!(messages.iter().any(|m| m.content == "How do I deploy?"));
    }

    #[test]
    fn user_events_paginated_uses_item_completed_channel() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            format!("{PAGINATED_META}\n{PAGINATED_DUAL_CHANNEL_BODY}"),
        )
        .expect("write");

        // The fork tree gets clean user events: no injected scaffolding
        // (app-context, AGENTS.md instructions wrapper, turn_aborted), no
        // twin duplication — but every real user turn shows up, including
        // one that merely opens with a scaffolding marker.
        let events = user_events_from_path(&path).expect("user_events");
        assert_eq!(
            events,
            vec!["How do I deploy?", "# AGENTS.md instructions say be nice"]
        );
    }

    #[test]
    fn user_events_paginated_fallback_keeps_legacy_semantics() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            format!("{PAGINATED_META}\n{LEGACY_RESPONSE_ITEM_BODY}"),
        )
        .expect("write");

        let events = user_events_from_path(&path).expect("user_events");
        assert_eq!(
            events,
            vec![
                "# AGENTS.md instructions for /tmp/project",
                "How do I deploy?"
            ]
        );
    }

    #[test]
    fn parse_session_paginated_title_prefers_item_completed_user_message() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            format!(
                "{PAGINATED_META}\n{}{}",
                concat!(
                    "{\"ordinal\":1,\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"# AGENTS.md instructions for /tmp/project\"}]}}\n",
                    "{\"ordinal\":2,\"timestamp\":\"2026-09-19T06:06:50.100Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"thread_id\":\"pag-id\",\"turn_id\":\"turn-1\",\"item\":{\"type\":\"UserMessage\",\"id\":\"u1\",\"client_id\":\"c1\",\"content\":[{\"type\":\"text\",\"text\":\"How do I deploy?\"}]},\"started_at_ms\":1,\"completed_at_ms\":1}}\n",
                ),
                ""
            ),
        )
        .expect("write");

        // The response_item channel only offers filtered-out injected context
        // in the head window; the item_completed user message still yields a
        // real title instead of falling through to the project basename.
        let meta = parse_session_with_titles(&path, &HashMap::new()).expect("parse");
        assert_eq!(meta.title.as_deref(), Some("How do I deploy?"));
    }

    #[test]
    fn parse_session_paginated_title_falls_back_to_response_item() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            format!(
                "{PAGINATED_META}\n{}",
                "{\"ordinal\":1,\"timestamp\":\"2026-09-19T06:06:50.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"How do I deploy?\"}]}}\n"
            ),
        )
        .expect("write");

        let meta = parse_session_with_titles(&path, &HashMap::new()).expect("parse");
        assert_eq!(meta.title.as_deref(), Some("How do I deploy?"));
    }

    #[test]
    fn parse_session_paginated_summary_prefers_item_completed() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            format!(
                "{PAGINATED_META}\n{}",
                concat!(
                    "{\"ordinal\":1,\"timestamp\":\"2026-09-19T06:06:53.000Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"RI final\"}]}}\n",
                    "{\"ordinal\":2,\"timestamp\":\"2026-09-19T06:06:54.000Z\",\"type\":\"event_msg\",\"payload\":{\"type\":\"item_completed\",\"thread_id\":\"pag-id\",\"turn_id\":\"turn-1\",\"item\":{\"type\":\"AgentMessage\",\"id\":\"msg_1\",\"content\":[{\"type\":\"Text\",\"text\":\"IC final\"}],\"phase\":\"final\"},\"started_at_ms\":1,\"completed_at_ms\":1}}\n",
                )
            ),
        )
        .expect("write");

        let meta = parse_session_with_titles(&path, &HashMap::new()).expect("parse");
        assert_eq!(meta.summary.as_deref(), Some("IC final"));
    }

    #[test]
    #[ignore = "manual regression: set CODEX_REAL_SAMPLE=<rollout.jsonl> and run with --ignored --nocapture"]
    fn real_sample_dual_channel_regression() {
        let Some(sample) = std::env::var_os("CODEX_REAL_SAMPLE") else {
            return;
        };
        let path = PathBuf::from(sample);
        let messages = load_messages(&path).expect("load_messages");
        let events = user_events_from_path(&path).expect("user_events");
        let meta = parse_session_with_titles(&path, &HashMap::new());

        // Channel-selection tripwires apply to paginated files only — legacy
        // rollouts carry a single message channel, and repeated text (e.g.
        // approval notices) is legitimate data there.
        let mut first_line = String::new();
        {
            use std::io::BufRead;
            let file = File::open(&path).expect("open sample");
            let mut reader = BufReader::new(file);
            reader.read_line(&mut first_line).expect("read sample");
        }
        let first_record: Value = serde_json::from_str(first_line.trim()).unwrap_or(Value::Null);
        let paginated = history_mode_from_record(&first_record) == HistoryMode::Paginated;

        if paginated {
            // Text twins land adjacently when both channels are mistakenly
            // merged. Identical "[Tool: name]" placeholders are exempt —
            // sequential calls to the same tool are legitimate repetition.
            for pair in messages.windows(2) {
                let duplicate = pair[0].role == pair[1].role && pair[0].content == pair[1].content;
                assert!(
                    !duplicate || pair[0].content.starts_with("[Tool: "),
                    "adjacent duplicate text message — channel selection leaked both twins: {:?}",
                    &pair[0].content.chars().take(60).collect::<String>()
                );
            }
            // On the paginated channel the first user message must be real
            // input, never injected context.
            if let Some(first_user) = messages.iter().find(|m| m.role == "user") {
                let trimmed = first_user.content.trim_start();
                assert!(
                    !trimmed.starts_with("# AGENTS.md")
                        && !trimmed.starts_with("<environment_context>"),
                    "first user message is injected context: {:?}",
                    trimmed.chars().take(60).collect::<String>()
                );
            }
        }
        // Derived tail-only files legitimately hold no user message at all;
        // only flag the case where user messages exist but events miss them.
        assert!(
            messages.iter().all(|m| m.role != "user") || !events.is_empty(),
            "user messages present but user events empty"
        );
        // Subagent rollouts are filtered from the session list by design.
        let title = meta.as_ref().and_then(|m| m.title.clone());
        println!(
            "messages={} user_events={} paginated={paginated} title={:?} meta={}",
            messages.len(),
            events.len(),
            title,
            if meta.is_some() {
                "parsed"
            } else {
                "filtered (subagent)"
            }
        );
    }
}
