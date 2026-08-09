use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use rusqlite::{Connection, OpenFlags, Row};
use serde_json::Value;

use crate::session_manager::types::ToolResultInfo;
use crate::session_manager::{
    SessionHandle, SessionLocator, SessionMessage, SessionMeta, ToolCallInfo,
};

use super::utils::{move_single_file, path_basename, truncate_summary, TITLE_MAX_CHARS};
use super::SessionProvider;

const PROVIDER_ID: &str = "opencode";

/// Cap on the number of SQLite sessions returned per list scan.
///
/// Prevents unbounded list materialization (plus downstream full-list
/// rendering and search indexing) from exhausting memory on very large
/// OpenCode databases. Deliberate stop-gap; a systematic refactor should
/// replace this with real pagination.
const DB_LIST_LIMIT: i64 = 1000;

// ─── OpenCodeProvider ───────────────────────────────────────────────────────

/// Provider implementation for OpenCode sessions.
///
/// Legacy storage layout:
///   {base}/storage/
///     session/{project_id}/{session_id}.json   — session metadata
///     message/{session_id}/{message_id}.json    — messages
///     part/{message_id}/{part_id}.json          — message parts
///
/// Newer OpenCode versions also store sessions in `{base}/opencode.db`.
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

    fn scan_roots(&self) -> Vec<PathBuf> {
        vec![
            crate::config::get_opencode_base_dir(),
            crate::config::get_opencode_archive_dir(),
        ]
    }

    fn scan_sessions(&self, root: &Path) -> Vec<SessionMeta> {
        scan_sessions_from_scan_root(root)
    }

    fn load_messages(&self, path: &Path) -> Result<Vec<SessionMessage>, String> {
        let storage_dir = derive_storage_base(path);
        load_messages_internal(path, &storage_dir)
    }

    fn load_messages_for_handle(
        &self,
        handle: &SessionHandle,
    ) -> Result<Vec<SessionMessage>, String> {
        let start = Instant::now();
        log::debug!(
            "opencode_message_load start session={} locator={} path={}",
            handle.session_id,
            handle.locator.detail_key_part(),
            handle.display_source_path()
        );
        match &handle.locator {
            SessionLocator::File { path } => {
                let result = self.load_messages(Path::new(path));
                match &result {
                    Ok(messages) => log::debug!(
                        "opencode_message_load finish session={} message_count={} elapsed_ms={}",
                        handle.session_id,
                        messages.len(),
                        start.elapsed().as_millis()
                    ),
                    Err(err) => log::warn!(
                        "opencode_message_load error session={} path={} elapsed_ms={} error={}",
                        handle.session_id,
                        handle.display_source_path(),
                        start.elapsed().as_millis(),
                        err
                    ),
                }
                result
            }
            SessionLocator::Database { path, record_id } => {
                let session_id = if record_id.is_empty() {
                    &handle.session_id
                } else {
                    record_id
                };
                let result = load_messages_from_db(Path::new(path), session_id);
                match &result {
                    Ok(messages) => log::debug!(
                        "opencode_message_load finish session={} message_count={} elapsed_ms={}",
                        handle.session_id,
                        messages.len(),
                        start.elapsed().as_millis()
                    ),
                    Err(err) => log::warn!(
                        "opencode_message_load error session={} path={} elapsed_ms={} error={}",
                        handle.session_id,
                        handle.display_source_path(),
                        start.elapsed().as_millis(),
                        err
                    ),
                }
                result
            }
        }
    }

    fn load_raw_content_fallback(&self, _path: &Path) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn load_raw_content_fallback_for_handle(
        &self,
        handle: &SessionHandle,
    ) -> Result<Option<String>, String> {
        match &handle.locator {
            SessionLocator::File { path } => self.load_raw_content_fallback(Path::new(path)),
            SessionLocator::Database { .. } => Ok(None),
        }
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

fn scan_sessions_from_scan_root(root: &Path) -> Vec<SessionMeta> {
    let start = Instant::now();
    let mut sessions = Vec::new();

    let db_path = if root.file_name().and_then(|name| name.to_str()) == Some("opencode.db") {
        root.to_path_buf()
    } else {
        root.join("opencode.db")
    };
    if db_path.is_file() {
        sessions.extend(scan_sessions_in_db(&db_path));
    }
    let mut seen_session_ids: HashSet<String> = sessions
        .iter()
        .map(|meta| meta.session_id.clone())
        .collect();

    let storage_root = if root.join("session").is_dir() {
        root.to_path_buf()
    } else {
        root.join("storage")
    };
    for meta in scan_sessions_in_legacy_storage(&storage_root) {
        if seen_session_ids.insert(meta.session_id.clone()) {
            sessions.push(meta);
        }
    }

    log::debug!(
        "opencode_scan finish root={} session_count={} elapsed_ms={}",
        root.display(),
        sessions.len(),
        start.elapsed().as_millis()
    );

    sessions
}

fn warn_opencode_scan(path: &Path, message: impl std::fmt::Display) {
    log::warn!(
        "opencode_scan warning path={} error={}",
        path.display(),
        message
    );
}

fn scan_sessions_in_legacy_storage(root: &Path) -> Vec<SessionMeta> {
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

fn open_db_readonly(db_path: &Path) -> Result<Connection, String> {
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Failed to open OpenCode database read-only: {e}"))?;
    conn.busy_timeout(std::time::Duration::from_millis(250))
        .map_err(|e| format!("Failed to set OpenCode database busy timeout: {e}"))?;
    Ok(conn)
}

fn scan_sessions_in_db(db_path: &Path) -> Vec<SessionMeta> {
    let start = Instant::now();
    let conn = match open_db_readonly(db_path) {
        Ok(conn) => conn,
        Err(err) => {
            warn_opencode_scan(db_path, err);
            return Vec::new();
        }
    };

    let summary_query = "SELECT id, title, directory, parent_id, time_created, time_updated, \
            model, agent, cost, tokens_input, tokens_output, \
            (SELECT json_extract(p.data, '$.text') \
               FROM message m \
               JOIN part p ON p.message_id = m.id \
              WHERE m.session_id = session.id \
                AND json_extract(p.data, '$.type') = 'text' \
                AND trim(coalesce(json_extract(p.data, '$.text'), '')) <> '' \
              ORDER BY m.time_created DESC, m.id DESC, p.time_created DESC, p.id DESC \
              LIMIT 1) AS preview_text \
     FROM session \
     ORDER BY time_updated DESC, id DESC \
     LIMIT ?";
    let session_only_query = "SELECT id, title, directory, parent_id, time_created, time_updated, \
            model, agent, cost, tokens_input, tokens_output, NULL AS preview_text \
     FROM session \
     ORDER BY time_updated DESC, id DESC \
     LIMIT ?";

    let mut stmt = match conn
        .prepare(summary_query)
        .or_else(|_| conn.prepare(session_only_query))
    {
        Ok(stmt) => stmt,
        Err(err) => {
            warn_opencode_scan(db_path, format!("failed to prepare session query: {err}"));
            return Vec::new();
        }
    };

    let rows = match stmt.query_map([DB_LIST_LIMIT], |row| db_session_row_to_meta(row, db_path)) {
        Ok(rows) => rows,
        Err(err) => {
            warn_opencode_scan(db_path, format!("failed to query session rows: {err}"));
            return Vec::new();
        }
    };

    let sessions: Vec<_> = rows
        .filter_map(|result| match result {
            Ok(meta) => Some(meta),
            Err(err) => {
                warn_opencode_scan(db_path, format!("failed to read session row: {err}"));
                None
            }
        })
        .collect();

    log::debug!(
        "opencode_db_scan finish path={} session_count={} elapsed_ms={}",
        db_path.display(),
        sessions.len(),
        start.elapsed().as_millis()
    );

    sessions
}

fn db_session_row_to_meta(row: &Row<'_>, db_path: &Path) -> rusqlite::Result<SessionMeta> {
    let session_id: String = row.get(0)?;
    let title: Option<String> = row.get(1)?;
    let directory: Option<String> = row.get(2)?;
    let parent_id: Option<String> = row.get(3)?;
    let created_at: Option<i64> = row.get(4)?;
    let updated_at: Option<i64> = row.get(5)?;
    let summary: Option<String> = row.get(11)?;
    let db_path_string = db_path.to_string_lossy().to_string();

    Ok(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title: title
            .filter(|s| !s.trim().is_empty())
            .or_else(|| directory.as_deref().and_then(path_basename))
            .map(|t| truncate_summary(&t, TITLE_MAX_CHARS)),
        summary: summary
            .filter(|s| !s.trim().is_empty())
            .map(|t| truncate_summary(&t, 160)),
        project_dir: directory,
        created_at,
        last_active_at: updated_at.or(created_at),
        source_path: Some(db_path_string.clone()),
        locator: Some(SessionLocator::Database {
            path: db_path_string,
            record_id: session_id.clone(),
        }),
        resume_command: Some(format!("opencode -s {session_id}")),
        forked_from_id: parent_id,
    })
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

    let first_user_message = first_user_message_summary(storage_dir, &session_id);
    let summary = latest_text_message_summary(storage_dir, &session_id);

    // Title priority: session JSON title > directory basename > first user message summary
    let title = title
        .or_else(|| directory.as_deref().and_then(path_basename))
        .or(first_user_message);

    // source_path points to the session JSON file itself, consistent with other providers
    // (Claude/Codex/Gemini all set source_path to the session file path)
    let source_path = path.to_string_lossy().to_string();

    Some(SessionMeta {
        provider_id: PROVIDER_ID.to_string(),
        session_id: session_id.clone(),
        title: title.map(|t| truncate_summary(&t, TITLE_MAX_CHARS)),
        summary: summary.map(|text| truncate_summary(&text, 160)),
        project_dir: directory,
        created_at,
        last_active_at,
        source_path: Some(source_path.clone()),
        locator: Some(SessionLocator::File { path: source_path }),
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

/// Read the latest text-bearing message for the compact list preview.
fn latest_text_message_summary(storage_dir: &Path, session_id: &str) -> Option<String> {
    let msg_dir = storage_dir.join("message").join(session_id);
    if !msg_dir.is_dir() {
        return None;
    }

    let mut messages: Vec<(i64, String)> = Vec::new();
    let entries = std::fs::read_dir(&msg_dir).ok()?;

    for entry in entries.flatten() {
        let msg_path = entry.path();
        if msg_path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let content = match std::fs::read_to_string(&msg_path) {
            Ok(content) => content,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&content) {
            Ok(value) => value,
            Err(_) => continue,
        };

        let created = value
            .get("time")
            .and_then(|t| t.get("created"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        let msg_id = match value.get("id").and_then(Value::as_str) {
            Some(id) => id.to_string(),
            None => continue,
        };

        let text = read_text_part_text(&storage_dir.join("part").join(&msg_id));
        if !text.trim().is_empty() {
            messages.push((created, text));
        }
    }

    messages.sort_by_key(|(ts, _)| *ts);
    messages.into_iter().next_back().map(|(_, text)| text)
}

/// Read only human-authored/generated text parts, excluding tool markers.
fn read_text_part_text(part_dir: &Path) -> String {
    read_part_values(part_dir)
        .into_iter()
        .filter_map(|value| {
            if value.get("type").and_then(Value::as_str) != Some("text") {
                return None;
            }
            value
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(str::to_string)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_part_values(part_dir: &Path) -> Vec<Value> {
    if !part_dir.is_dir() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(part_dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };

    let mut paths: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    paths.sort();

    paths
        .into_iter()
        .filter_map(|part_path| {
            let content = std::fs::read_to_string(&part_path).ok()?;
            serde_json::from_str(&content).ok()
        })
        .collect()
}

/// Read all text parts from a part directory and join them.
/// Entries are sorted by filename to ensure deterministic ordering across
/// platforms (Linux ext4 does not guarantee alphabetical readdir order).
fn read_part_text(part_dir: &Path) -> String {
    let mut parts = Vec::new();
    for value in read_part_values(part_dir) {
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

/// Extract the tool result output from an OpenCode tool part.
///
/// OpenCode stores the result on the same part as the call: the output lives in
/// `state.output` (older data duplicates it in `state.metadata.output`). A
/// non-string or empty output (e.g. a failed skill call) yields `None`.
fn extract_tool_output(value: &Value) -> Option<String> {
    let state = value.get("state")?;
    let from = |v: &Value| {
        v.as_str()
            .map(str::to_string)
            .filter(|s| !s.trim().is_empty())
    };
    state.get("output").and_then(from).or_else(|| {
        state
            .get("metadata")
            .and_then(|m| m.get("output"))
            .and_then(from)
    })
}

/// Read tool result outputs from the parts directory.
/// Mirrors `read_part_tool_calls` for the file-backed storage layout.
fn read_part_tool_results(part_dir: &Path) -> Option<ToolResultInfo> {
    if !part_dir.is_dir() {
        return None;
    }

    let entries = match std::fs::read_dir(part_dir) {
        Ok(entries) => entries,
        Err(_) => return None,
    };

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    paths.sort();

    let mut result = None;
    for part_path in paths {
        let content = match std::fs::read_to_string(&part_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let value: Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if value.get("type").and_then(Value::as_str) != Some("tool") {
            continue;
        }
        if let Some(output) = extract_tool_output(&value) {
            let call_id = value
                .get("callID")
                .and_then(Value::as_str)
                .map(str::to_string);
            result = Some(ToolResultInfo {
                content: output,
                call_id,
            });
        }
    }

    result
}

struct DbMessageDraft {
    id: String,
    role: String,
    created: i64,
    content_parts: Vec<String>,
    tool_calls: Vec<ToolCallInfo>,
    tool_result: Option<ToolResultInfo>,
}

impl DbMessageDraft {
    fn from_row(message_id: String, created: Option<i64>, data: Option<String>) -> Self {
        let value = data
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok());
        let role = value
            .as_ref()
            .and_then(|v| v.get("role"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        Self {
            id: message_id,
            role,
            created: created.unwrap_or(0),
            content_parts: Vec::new(),
            tool_calls: Vec::new(),
            tool_result: None,
        }
    }

    fn push_part(&mut self, part_data: Option<String>) {
        let Some(raw) = part_data else {
            return;
        };
        let Ok(value) = serde_json::from_str::<Value>(&raw) else {
            return;
        };

        match value.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(text) = value
                    .get("text")
                    .or_else(|| value.get("content"))
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                {
                    self.content_parts.push(text.to_string());
                }
            }
            Some("tool") => {
                let call_id = value
                    .get("callID")
                    .or_else(|| value.get("call_id"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let Some(name) = value
                    .get("tool")
                    .or_else(|| value.get("name"))
                    .and_then(Value::as_str)
                {
                    self.content_parts.push(format!("[Tool: {name}]"));
                    self.tool_calls.push(ToolCallInfo {
                        name: name.to_string(),
                        input: value
                            .get("state")
                            .and_then(|state| state.get("input"))
                            .map(|input| input.to_string())
                            .unwrap_or_default(),
                        call_id: call_id.clone(),
                    });
                }
                // OpenCode stores the tool result on the same part as the call.
                // Keep the last non-empty output; a single message maps to one
                // ToolResultInfo in the shared SessionMessage model.
                if let Some(output) = extract_tool_output(&value) {
                    self.tool_result = Some(ToolResultInfo {
                        content: output,
                        call_id,
                    });
                }
            }
            Some("file") => {
                if let Some(path) = value
                    .get("path")
                    .or_else(|| value.get("filename"))
                    .or_else(|| value.get("name"))
                    .and_then(Value::as_str)
                {
                    self.content_parts.push(format!("[File: {path}]"));
                }
            }
            _ => {}
        }
    }

    fn into_message(self) -> Option<SessionMessage> {
        let content = self.content_parts.join("\n");
        if content.trim().is_empty() && self.tool_result.is_none() {
            return None;
        }

        Some(SessionMessage {
            role: self.role,
            content,
            ts: Some(self.created),
            usage: None,
            cumulative_usage: None,
            tool_calls: if self.tool_calls.is_empty() {
                None
            } else {
                Some(self.tool_calls)
            },
            tool_result: self.tool_result,
        })
    }
}

fn load_messages_from_db(db_path: &Path, session_id: &str) -> Result<Vec<SessionMessage>, String> {
    let start = Instant::now();
    log::debug!(
        "opencode_db_detail start path={} session={}",
        db_path.display(),
        session_id
    );
    let conn = open_db_readonly(db_path)?;
    let part_join = if part_table_has_session_id(&conn)? {
        "LEFT JOIN part p ON p.message_id = m.id AND p.session_id = m.session_id"
    } else {
        "LEFT JOIN part p ON p.message_id = m.id"
    };
    let query = format!(
        "SELECT m.id, m.time_created, m.data, p.id, p.time_created, p.data \
         FROM message m \
         {part_join} \
         WHERE m.session_id = ? \
         ORDER BY m.time_created ASC, m.id ASC, p.time_created ASC, p.id ASC"
    );
    let mut stmt = conn
        .prepare(&query)
        .map_err(|e| format!("Failed to prepare OpenCode message query: {e}"))?;

    let mut rows = stmt
        .query([session_id])
        .map_err(|e| format!("Failed to query OpenCode messages: {e}"))?;
    let mut drafts = Vec::new();
    let mut current: Option<DbMessageDraft> = None;
    let mut part_row_count = 0usize;

    while let Some(row) = rows
        .next()
        .map_err(|e| format!("Failed to read OpenCode message row: {e}"))?
    {
        let part_id: Option<String> = row.get(3).ok();
        if part_id.is_some() {
            part_row_count += 1;
        }

        let message_id: String = row
            .get(0)
            .map_err(|e| format!("OpenCode message row is missing id: {e}"))?;

        if current.as_ref().map(|draft| draft.id.as_str()) != Some(message_id.as_str()) {
            if let Some(draft) = current.take() {
                drafts.push(draft);
            }
            current = Some(DbMessageDraft::from_row(
                message_id,
                row.get(1).ok(),
                row.get(2).ok(),
            ));
        }

        if let Some(draft) = current.as_mut() {
            draft.push_part(row.get(5).ok());
        }
    }

    if let Some(draft) = current {
        drafts.push(draft);
    }

    let messages: Vec<_> = drafts
        .into_iter()
        .filter_map(DbMessageDraft::into_message)
        .collect();

    log::debug!(
        "opencode_db_detail finish path={} session={} message_count={} part_row_count={} elapsed_ms={}",
        db_path.display(),
        session_id,
        messages.len(),
        part_row_count,
        start.elapsed().as_millis()
    );

    Ok(messages)
}

fn part_table_has_session_id(conn: &Connection) -> Result<bool, String> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(part)")
        .map_err(|e| format!("Failed to inspect OpenCode part schema: {e}"))?;
    let mut rows = stmt
        .query([])
        .map_err(|e| format!("Failed to query OpenCode part schema: {e}"))?;

    while let Some(row) = rows
        .next()
        .map_err(|e| format!("Failed to read OpenCode part schema: {e}"))?
    {
        let name: String = row
            .get(1)
            .map_err(|e| format!("Failed to read OpenCode part column name: {e}"))?;
        if name == "session_id" {
            return Ok(true);
        }
    }

    Ok(false)
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

    let mut messages_raw: Vec<(
        i64,
        String,
        String,
        Vec<ToolCallInfo>,
        Option<ToolResultInfo>,
    )> = Vec::new();

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
        let tool_calls = read_part_tool_calls(&part_dir);
        let tool_result = read_part_tool_results(&part_dir);
        let text = read_part_text(&part_dir);
        if text.trim().is_empty() && tool_calls.is_empty() && tool_result.is_none() {
            continue;
        }

        messages_raw.push((created, role, text, tool_calls, tool_result));
    }

    // Sort by created timestamp
    messages_raw.sort_by_key(|(ts, _, _, _, _)| *ts);

    Ok(messages_raw
        .into_iter()
        .map(|(ts, role, content, tool_calls, tool_result)| {
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
                tool_result,
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
    use crate::config::TEST_ENV_LOCK;
    use crate::session_manager::{
        archive_session_for_handle, build_provider_registry, load_session_detail_for_handle,
    };
    use rusqlite::params;
    use tempfile::tempdir;

    static ENV_LOCK: &std::sync::Mutex<()> = &TEST_ENV_LOCK;

    struct EnvVarGuard {
        key: &'static str,
        old_value: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set_path(key: &'static str, value: &Path) -> Self {
            let old_value = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, old_value }
        }

        fn remove(key: &'static str) -> Self {
            let old_value = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, old_value }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.old_value {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

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

    /// Write a tool part with real-world fields: callID + state.input + state.output.
    fn write_tool_part_with_call(
        storage: &Path,
        msg_id: &str,
        part_id: &str,
        tool: &str,
        call_id: &str,
        command: &str,
        output: &str,
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
                "output": output,
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

    fn write_sqlite_fixture(base: &Path) -> PathBuf {
        std::fs::create_dir_all(base).expect("create opencode base");
        let db_path = base.join("opencode.db");
        let conn = Connection::open(&db_path).expect("open sqlite fixture");
        conn.execute_batch(
            r#"
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                parent_id TEXT,
                time_created INTEGER,
                time_updated INTEGER,
                model TEXT,
                agent TEXT,
                cost REAL,
                tokens_input INTEGER,
                tokens_output INTEGER
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER,
                data TEXT
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                time_created INTEGER,
                data TEXT
            );
            "#,
        )
        .expect("create sqlite schema");

        conn.execute(
            "INSERT INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "ses_db_1",
                "SQLite Session",
                "/tmp/opencode-project",
                Option::<String>::None,
                1_740_000_000_000i64,
                1_740_000_020_000i64,
                "model-a",
                "agent-a",
                0.01f64,
                12i64,
                34i64
            ],
        )
        .expect("insert session one");
        conn.execute(
            "INSERT INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "ses_db_2",
                Option::<String>::None,
                "/tmp/other-project",
                "ses_db_1",
                1_740_000_001_000i64,
                1_740_000_010_000i64,
                "model-b",
                "agent-b",
                0.02f64,
                1i64,
                2i64
            ],
        )
        .expect("insert session two");

        conn.execute(
            "INSERT INTO message VALUES (?1, ?2, ?3, ?4)",
            params![
                "msg_user",
                "ses_db_1",
                1_740_000_002_000i64,
                serde_json::json!({"role":"user"}).to_string()
            ],
        )
        .expect("insert user message");
        conn.execute(
            "INSERT INTO message VALUES (?1, ?2, ?3, ?4)",
            params![
                "msg_assistant",
                "ses_db_1",
                1_740_000_003_000i64,
                serde_json::json!({"role":"assistant"}).to_string()
            ],
        )
        .expect("insert assistant message");
        conn.execute(
            "INSERT INTO message VALUES (?1, ?2, ?3, ?4)",
            params![
                "msg_other",
                "ses_db_2",
                1_740_000_004_000i64,
                serde_json::json!({"role":"user"}).to_string()
            ],
        )
        .expect("insert other message");

        conn.execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "part_user_text",
                "msg_user",
                "ses_db_1",
                1_740_000_002_100i64,
                serde_json::json!({"type":"text","text":"Hello from SQLite"}).to_string()
            ],
        )
        .expect("insert user text part");
        conn.execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "part_assistant_text",
                "msg_assistant",
                "ses_db_1",
                1_740_000_003_100i64,
                serde_json::json!({"type":"text","text":"Let me inspect"}).to_string()
            ],
        )
        .expect("insert assistant text part");
        conn.execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "part_assistant_tool",
                "msg_assistant",
                "ses_db_1",
                1_740_000_003_200i64,
                serde_json::json!({
                    "type":"tool",
                    "tool":"bash",
                    "callID":"call_sqlite_1",
                    "state":{
                        "input":{"command":"pwd"},
                        "output":"/home/user"
                    }
                })
                .to_string()
            ],
        )
        .expect("insert tool part");
        conn.execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "part_unknown",
                "msg_assistant",
                "ses_db_1",
                1_740_000_003_300i64,
                serde_json::json!({"type":"unknown","value":"skip me"}).to_string()
            ],
        )
        .expect("insert unknown part");
        conn.execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "part_other_text",
                "msg_other",
                "ses_db_2",
                1_740_000_004_100i64,
                serde_json::json!({"type":"text","text":"Other session"}).to_string()
            ],
        )
        .expect("insert other part");

        db_path
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
        assert_eq!(provider.scan_roots().len(), 2);
        assert!(
            provider.roots()[0].ends_with("storage"),
            "operation root should remain the legacy storage directory"
        );
        assert!(
            !provider.scan_roots()[0].ends_with("storage"),
            "scan root should widen to the OpenCode base directory"
        );
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
    fn sqlite_scan_sets_summary_from_latest_text_part() {
        let temp = tempdir().expect("tempdir");
        let _db_path = write_sqlite_fixture(temp.path());

        let sessions = scan_sessions_from_scan_root(temp.path());
        let session = sessions
            .iter()
            .find(|session| session.session_id == "ses_db_1")
            .expect("sqlite session");

        assert_eq!(session.summary.as_deref(), Some("Let me inspect"));
    }

    #[test]
    fn parse_legacy_session_sets_summary_from_latest_text_message() {
        let ts = TestStorage::new();
        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_summary",
            Some("Summary Session"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );
        write_message(
            &ts.storage,
            "ses_summary",
            "msg_1",
            "user",
            1_740_000_001_000,
        );
        write_text_part(&ts.storage, "msg_1", "prt_1", "First prompt");
        write_message(
            &ts.storage,
            "ses_summary",
            "msg_2",
            "assistant",
            1_740_000_002_000,
        );
        write_text_part(&ts.storage, "msg_2", "prt_2", "Latest answer");

        let provider = OpenCodeProvider;
        let meta = provider
            .parse_session(&session_path)
            .expect("parse session");

        assert_eq!(meta.summary.as_deref(), Some("Latest answer"));
    }

    #[test]
    fn parse_legacy_session_title_fallback_uses_first_user_not_latest_summary() {
        let ts = TestStorage::new();
        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_title_fallback",
            None,
            None,
            1_740_000_000_000,
            None,
        );
        write_message(
            &ts.storage,
            "ses_title_fallback",
            "msg_1",
            "user",
            1_740_000_001_000,
        );
        write_text_part(&ts.storage, "msg_1", "prt_1", "Opening question");
        write_message(
            &ts.storage,
            "ses_title_fallback",
            "msg_2",
            "assistant",
            1_740_000_002_000,
        );
        write_text_part(&ts.storage, "msg_2", "prt_2", "Latest answer");

        let provider = OpenCodeProvider;
        let meta = provider
            .parse_session(&session_path)
            .expect("parse session");

        assert_eq!(meta.title.as_deref(), Some("Opening question"));
        assert_eq!(meta.summary.as_deref(), Some("Latest answer"));
    }

    #[test]
    fn parse_legacy_session_summary_skips_tool_only_latest_message() {
        let ts = TestStorage::new();
        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_tool_summary",
            Some("Tool Summary"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );
        write_message(
            &ts.storage,
            "ses_tool_summary",
            "msg_1",
            "assistant",
            1_740_000_001_000,
        );
        write_text_part(&ts.storage, "msg_1", "prt_1", "Readable answer");
        write_message(
            &ts.storage,
            "ses_tool_summary",
            "msg_2",
            "assistant",
            1_740_000_002_000,
        );
        write_tool_part(&ts.storage, "msg_2", "prt_2", "Write");

        let provider = OpenCodeProvider;
        let meta = provider
            .parse_session(&session_path)
            .expect("parse session");

        assert_eq!(meta.summary.as_deref(), Some("Readable answer"));
    }

    #[test]
    fn move_session_moves_file() {
        let temp = tempdir().expect("tempdir");
        let source_file = temp.path().join("session-test.json");
        std::fs::write(&source_file, r#"{"id":"move-test"}"#).expect("write");
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
    fn scan_sessions_reads_sqlite_database() {
        let temp = tempdir().expect("tempdir");
        let db_path = write_sqlite_fixture(temp.path());

        let sessions = scan_sessions_from_scan_root(temp.path());

        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "ses_db_1");
        assert_eq!(sessions[0].title.as_deref(), Some("SQLite Session"));
        assert_eq!(
            sessions[0].source_path.as_deref(),
            Some(db_path.to_string_lossy().as_ref())
        );
        assert_eq!(
            sessions[0].locator,
            Some(SessionLocator::Database {
                path: db_path.to_string_lossy().to_string(),
                record_id: "ses_db_1".to_string(),
            })
        );
        assert_eq!(sessions[1].title.as_deref(), Some("other-project"));
        assert_eq!(sessions[1].forked_from_id.as_deref(), Some("ses_db_1"));
    }

    #[test]
    fn scan_sessions_in_db_respects_list_limit() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join("opencode.db");
        let conn = Connection::open(&db_path).expect("open sqlite");
        conn.execute_batch(
            r#"
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                parent_id TEXT,
                time_created INTEGER,
                time_updated INTEGER,
                model TEXT,
                agent TEXT,
                cost REAL,
                tokens_input INTEGER,
                tokens_output INTEGER
            );
            "#,
        )
        .expect("create sqlite schema");

        let count = DB_LIST_LIMIT + 1;
        for i in 0..count {
            conn.execute(
                "INSERT INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    format!("ses_{i:05}"),
                    Some(format!("Session {i}")),
                    "/tmp/proj",
                    Option::<String>::None,
                    i,
                    i, // time_updated = i, so higher i is more recent
                    "model-a",
                    "agent-a",
                    0.0f64,
                    1i64,
                    2i64
                ],
            )
            .expect("insert session");
        }
        drop(conn);

        let sessions = scan_sessions_from_scan_root(temp.path());
        assert_eq!(sessions.len(), DB_LIST_LIMIT as usize);
        assert!(
            sessions
                .iter()
                .any(|s| s.session_id == format!("ses_{:05}", count - 1)),
            "the most recent session must be within the capped list"
        );
    }

    #[test]
    fn scan_sessions_in_db_tie_breaks_same_timestamp_by_id() {
        // All sessions share one time_updated. With the id tie-breaker the
        // LIMIT cutoff is deterministic: exactly the highest ids are kept.
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join("opencode.db");
        let conn = Connection::open(&db_path).expect("open sqlite");
        conn.execute_batch(
            r#"
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                parent_id TEXT,
                time_created INTEGER,
                time_updated INTEGER,
                model TEXT,
                agent TEXT,
                cost REAL,
                tokens_input INTEGER,
                tokens_output INTEGER
            );
            "#,
        )
        .expect("create sqlite schema");

        let count = DB_LIST_LIMIT + 20;
        for i in 0..count {
            conn.execute(
                "INSERT INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    format!("ses_{i:05}"),
                    Option::<String>::None,
                    "/tmp/proj",
                    Option::<String>::None,
                    0i64,
                    1_740_000_000_000i64, // identical time_updated for all rows
                    "model-a",
                    "agent-a",
                    0.0f64,
                    1i64,
                    2i64
                ],
            )
            .expect("insert session");
        }
        drop(conn);

        let sessions = scan_sessions_from_scan_root(temp.path());
        assert_eq!(sessions.len(), DB_LIST_LIMIT as usize);

        // id is zero-padded so string order equals numeric order: the kept rows
        // are exactly the DB_LIST_LIMIT highest ids, in descending id order.
        let ids: Vec<i64> = sessions
            .iter()
            .map(|s| s.session_id[4..].parse::<i64>().expect("numeric id"))
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(
            ids, sorted,
            "rows must be ordered by id DESC on timestamp ties"
        );
        assert_eq!(ids[0], count - 1, "highest id must be first");
        assert_eq!(
            ids[ids.len() - 1],
            count - DB_LIST_LIMIT,
            "cutoff must be exact"
        );
    }

    #[test]
    fn load_messages_reads_selected_sqlite_session() {
        let temp = tempdir().expect("tempdir");
        let db_path = write_sqlite_fixture(temp.path());
        let provider = OpenCodeProvider;
        let handle = SessionHandle {
            provider_id: "opencode".to_string(),
            session_id: "ses_db_1".to_string(),
            locator: SessionLocator::Database {
                path: db_path.to_string_lossy().to_string(),
                record_id: "ses_db_1".to_string(),
            },
        };

        let messages = provider
            .load_messages_for_handle(&handle)
            .expect("load sqlite messages");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello from SQLite");
        assert_eq!(messages[0].ts, Some(1_740_000_002_000));
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "Let me inspect\n[Tool: bash]");
        assert!(!messages[1].content.contains("Other session"));

        let calls = messages[1].tool_calls.as_ref().expect("tool calls");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "bash");
        assert_eq!(calls[0].call_id.as_deref(), Some("call_sqlite_1"));
        assert!(calls[0].input.contains("pwd"));

        // OpenCode stores the result on the same part: state.output
        let result = messages[1].tool_result.as_ref().expect("tool result");
        assert_eq!(result.content, "/home/user");
        assert_eq!(result.call_id.as_deref(), Some("call_sqlite_1"));
    }

    #[test]
    fn load_messages_ignores_parts_from_other_sessions_with_same_message_id() {
        let temp = tempdir().expect("tempdir");
        std::fs::create_dir_all(temp.path()).expect("create temp root");
        let db_path = temp.path().join("opencode.db");
        let conn = Connection::open(&db_path).expect("open sqlite fixture");
        conn.execute_batch(
            r#"
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                parent_id TEXT,
                time_created INTEGER,
                time_updated INTEGER,
                model TEXT,
                agent TEXT,
                cost REAL,
                tokens_input INTEGER,
                tokens_output INTEGER
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER,
                data TEXT
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL,
                session_id TEXT NOT NULL,
                time_created INTEGER,
                data TEXT
            );
            "#,
        )
        .expect("create sqlite schema");

        conn.execute(
            "INSERT INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "ses_a",
                "A",
                "/tmp/a",
                Option::<String>::None,
                1_740_000_000_000i64,
                1_740_000_020_000i64,
                "model-a",
                "agent-a",
                0.01f64,
                12i64,
                34i64
            ],
        )
        .expect("insert session a");
        conn.execute(
            "INSERT INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "ses_b",
                "B",
                "/tmp/b",
                Option::<String>::None,
                1_740_000_001_000i64,
                1_740_000_021_000i64,
                "model-b",
                "agent-b",
                0.02f64,
                1i64,
                2i64
            ],
        )
        .expect("insert session b");
        conn.execute(
            "INSERT INTO message VALUES (?1, ?2, ?3, ?4)",
            params![
                "shared_msg",
                "ses_a",
                1_740_000_002_000i64,
                serde_json::json!({"role":"user"}).to_string()
            ],
        )
        .expect("insert message a");
        conn.execute(
            "INSERT INTO message VALUES (?1, ?2, ?3, ?4)",
            params![
                "msg_b",
                "ses_b",
                1_740_000_003_000i64,
                serde_json::json!({"role":"user"}).to_string()
            ],
        )
        .expect("insert message b");
        conn.execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "part_a",
                "shared_msg",
                "ses_a",
                1_740_000_002_100i64,
                serde_json::json!({"type":"text","text":"A content"}).to_string()
            ],
        )
        .expect("insert part a");
        conn.execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "part_b",
                "msg_b",
                "ses_b",
                1_740_000_003_100i64,
                serde_json::json!({"type":"text","text":"B content"}).to_string()
            ],
        )
        .expect("insert part b");
        conn.execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                "part_bad",
                "shared_msg",
                "ses_b",
                1_740_000_003_200i64,
                serde_json::json!({"type":"text","text":"BAD content"}).to_string()
            ],
        )
        .expect("insert stray part");

        let messages_a = super::load_messages_from_db(&db_path, "ses_a").expect("load messages a");
        let messages_b = super::load_messages_from_db(&db_path, "ses_b").expect("load messages b");

        assert_eq!(messages_a.len(), 1);
        assert_eq!(messages_a[0].content, "A content");
        assert_eq!(messages_b.len(), 1);
        assert_eq!(messages_b[0].content, "B content");
    }

    #[test]
    fn load_messages_supports_sqlite_part_schema_without_session_id() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join("opencode.db");
        let conn = Connection::open(&db_path).expect("open sqlite fixture");
        conn.execute_batch(
            r#"
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT,
                directory TEXT,
                parent_id TEXT,
                time_created INTEGER,
                time_updated INTEGER,
                model TEXT,
                agent TEXT,
                cost REAL,
                tokens_input INTEGER,
                tokens_output INTEGER
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER,
                data TEXT
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL,
                time_created INTEGER,
                data TEXT
            );
            "#,
        )
        .expect("create sqlite schema");

        conn.execute(
            "INSERT INTO session VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                "ses_legacy_db",
                "Legacy DB",
                "/tmp/legacy-db",
                Option::<String>::None,
                1_740_000_000_000i64,
                1_740_000_020_000i64,
                "model-a",
                "agent-a",
                0.01f64,
                12i64,
                34i64
            ],
        )
        .expect("insert session");
        conn.execute(
            "INSERT INTO message VALUES (?1, ?2, ?3, ?4)",
            params![
                "msg_legacy",
                "ses_legacy_db",
                1_740_000_002_000i64,
                serde_json::json!({"role":"user"}).to_string()
            ],
        )
        .expect("insert message");
        conn.execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4)",
            params![
                "part_legacy",
                "msg_legacy",
                1_740_000_002_100i64,
                serde_json::json!({"type":"text","text":"Legacy schema content"}).to_string()
            ],
        )
        .expect("insert part");
        drop(conn);

        let messages = super::load_messages_from_db(&db_path, "ses_legacy_db")
            .expect("load legacy sqlite messages");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "Legacy schema content");
    }

    #[test]
    fn load_session_detail_reads_sqlite_session_through_registry() {
        let temp = tempdir().expect("tempdir");
        let db_path = write_sqlite_fixture(temp.path());
        let registry = build_provider_registry();
        let handle = SessionHandle {
            provider_id: "opencode".to_string(),
            session_id: "ses_db_1".to_string(),
            locator: SessionLocator::Database {
                path: db_path.to_string_lossy().to_string(),
                record_id: "ses_db_1".to_string(),
            },
        };

        let detail = load_session_detail_for_handle(&registry, &handle).expect("load detail");

        assert_eq!(detail.messages.len(), 2);
        assert_eq!(detail.qa_pairs.len(), 1);
        assert_eq!(detail.raw_content, None);
    }

    #[test]
    fn sqlite_raw_content_fallback_returns_none() {
        let temp = tempdir().expect("tempdir");
        let db_path = write_sqlite_fixture(temp.path());
        let provider = OpenCodeProvider;
        let handle = SessionHandle {
            provider_id: "opencode".to_string(),
            session_id: "ses_db_empty".to_string(),
            locator: SessionLocator::Database {
                path: db_path.to_string_lossy().to_string(),
                record_id: "ses_db_empty".to_string(),
            },
        };

        let raw = provider
            .load_raw_content_fallback_for_handle(&handle)
            .expect("db raw fallback");

        assert!(raw.is_none());
    }

    #[test]
    fn scan_sessions_prefers_sqlite_when_legacy_has_same_session_id() {
        let temp = tempdir().expect("tempdir");
        let db_path = write_sqlite_fixture(temp.path());
        let legacy_storage = temp.path().join("storage");
        write_session(
            &legacy_storage,
            "proj_abc",
            "ses_db_1",
            Some("Legacy Duplicate"),
            Some("/tmp/legacy"),
            1_740_000_000_000,
            None,
        );

        let sessions = scan_sessions_from_scan_root(temp.path());

        let matching: Vec<_> = sessions
            .iter()
            .filter(|session| session.session_id == "ses_db_1")
            .collect();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].title.as_deref(), Some("SQLite Session"));
        assert_eq!(
            matching[0].locator,
            Some(SessionLocator::Database {
                path: db_path.to_string_lossy().to_string(),
                record_id: "ses_db_1".to_string(),
            })
        );
    }

    #[test]
    fn scan_sessions_logs_and_skips_unreadable_sqlite_schema() {
        let temp = tempdir().expect("tempdir");
        let db_path = temp.path().join("opencode.db");
        let conn = Connection::open(&db_path).expect("open sqlite fixture");
        conn.execute_batch("CREATE TABLE not_session (id TEXT PRIMARY KEY);")
            .expect("create incompatible schema");
        drop(conn);

        let sessions = scan_sessions_from_scan_root(temp.path());

        assert!(sessions.is_empty());
    }

    #[test]
    fn archive_legacy_session_uses_storage_operation_root() {
        let _guard = ENV_LOCK.lock().expect("lock");
        let test_home = tempdir().expect("tempdir");
        let _test_home_guard = EnvVarGuard::set_path("SESSION_MANAGER_TEST_HOME", test_home.path());
        let _xdg_data_home_guard = EnvVarGuard::remove("XDG_DATA_HOME");

        let storage = test_home.path().join(".local/share/opencode/storage");
        let session_path = write_session(
            &storage,
            "proj_abc",
            "ses_archive_root",
            Some("Archive Root"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );
        let registry = build_provider_registry();
        let handle = SessionHandle {
            provider_id: "opencode".to_string(),
            session_id: "ses_archive_root".to_string(),
            locator: SessionLocator::File {
                path: session_path.to_string_lossy().to_string(),
            },
        };

        archive_session_for_handle(&registry, &handle).expect("archive legacy opencode session");

        assert!(
            !session_path.exists(),
            "legacy session should move out of active storage"
        );
        assert!(
            test_home
                .path()
                .join(
                    ".local/share/opencode/storage_archived/session/proj_abc/ses_archive_root.json"
                )
                .exists(),
            "archive path should be relative to storage, without an extra storage segment"
        );
        assert!(
            !test_home
                .path()
                .join(".local/share/opencode/storage_archived/storage/session/proj_abc/ses_archive_root.json")
                .exists(),
            "archive path must not include a duplicated storage segment"
        );
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
        // Tool part with real-world callID + state.input.command + state.output
        write_tool_part_with_call(
            &ts.storage,
            "msg_t2",
            "prt_t3",
            "bash",
            "call_79398764692c484892159dad",
            "python -m venv venv",
            "venv created",
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

        // Tool result comes from the same part's state.output
        let result = assistant
            .tool_result
            .as_ref()
            .expect("should have tool_result");
        assert_eq!(result.content, "venv created");
        assert_eq!(
            result.call_id.as_deref(),
            Some("call_79398764692c484892159dad")
        );
    }

    #[test]
    fn load_messages_keeps_tool_result_only_legacy_parts() {
        let ts = TestStorage::new();
        let session_path = write_session(
            &ts.storage,
            "proj_abc",
            "ses_tool_only",
            Some("Tool Result Only"),
            Some("/tmp"),
            1_740_000_000_000,
            None,
        );

        write_message(
            &ts.storage,
            "ses_tool_only",
            "msg_t1",
            "assistant",
            1_740_000_001_000,
        );
        let part_dir = ts.storage.join("part").join("msg_t1");
        std::fs::create_dir_all(&part_dir).expect("create part dir");
        let part_early = serde_json::json!({
            "type": "tool",
            "callID": "call_tool_only",
            "state": { "output": "first" }
        });
        let part_late = serde_json::json!({
            "type": "tool",
            "callID": "call_tool_only",
            "state": { "output": "tool finished" }
        });
        std::fs::write(
            part_dir.join("prt_a.json"),
            serde_json::to_string_pretty(&part_early).expect("serialize"),
        )
        .expect("write part");
        std::fs::write(
            part_dir.join("prt_b.json"),
            serde_json::to_string_pretty(&part_late).expect("serialize"),
        )
        .expect("write part");

        let provider = OpenCodeProvider;
        let meta = provider
            .parse_session(&session_path)
            .expect("parse session");
        let sp = meta.source_path.expect("source_path");
        let messages = provider
            .load_messages(std::path::Path::new(&sp))
            .expect("load_messages");

        assert_eq!(messages.len(), 1);
        assert!(messages[0].content.trim().is_empty());
        let result = messages[0].tool_result.as_ref().expect("tool_result");
        assert_eq!(result.content, "tool finished");
        assert_eq!(result.call_id.as_deref(), Some("call_tool_only"));
    }

    #[test]
    fn extract_tool_output_reads_metadata_output_when_state_output_missing() {
        let value = serde_json::json!({
            "type": "tool",
            "state": { "metadata": { "output": "fallback" } }
        });
        assert_eq!(extract_tool_output(&value).as_deref(), Some("fallback"));
    }

    #[test]
    fn extract_tool_output_reads_state_output() {
        let value = serde_json::json!({
            "type": "tool",
            "tool": "bash",
            "state": { "output": "done", "metadata": { "output": "stale" } }
        });
        assert_eq!(extract_tool_output(&value).as_deref(), Some("done"));
    }

    #[test]
    fn extract_tool_output_falls_back_to_metadata_output() {
        let value = serde_json::json!({
            "type": "tool",
            "state": { "output": "", "metadata": { "output": "fallback" } }
        });
        assert_eq!(extract_tool_output(&value).as_deref(), Some("fallback"));
    }

    #[test]
    fn extract_tool_output_returns_none_when_empty_or_missing() {
        // No state at all
        assert_eq!(
            extract_tool_output(&serde_json::json!({"type":"tool"})),
            None
        );
        // Empty output everywhere
        let value = serde_json::json!({
            "type": "tool",
            "state": { "output": "", "metadata": { "output": "" } }
        });
        assert_eq!(extract_tool_output(&value), None);
        // Non-string output
        let value = serde_json::json!({"type":"tool","state":{"output":{}}});
        assert_eq!(extract_tool_output(&value), None);
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

        let sessions = scan_sessions_from_scan_root(storage_dir);
        assert_eq!(sessions.len(), 2);
        let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        assert!(ids.contains(&"ses_001"));
        assert!(ids.contains(&"ses_002"));
    }

    #[test]
    fn scan_sessions_empty_dir_returns_empty() {
        let ts = TestStorage::new();
        let sessions = scan_sessions_from_scan_root(&ts.storage);
        assert!(sessions.is_empty());
    }

    #[test]
    fn scan_sessions_ignores_non_session_files_in_root() {
        let ts = TestStorage::new();
        std::fs::write(ts.storage.join("random.json"), "{}").expect("write random");

        let sessions = scan_sessions_from_scan_root(&ts.storage);
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
