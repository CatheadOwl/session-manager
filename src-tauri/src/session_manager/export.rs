//! Time-ranged Q&A session export — the single logic site for the export
//! capability (workunit 20260909-1110-qa-session-export-service).
//!
//! Adapters (Tauri command, future CLI/MCP) must only translate parameters
//! and write/return the rendered output; filtering, distilling, and
//! provenance assembly happen here and nowhere else.

use std::path::Path;

use super::messages::load_messages_for_handle;
use super::providers::ProviderRegistry;
use super::scan::scan_sessions_with_scope;
use super::types::{
    ExportSkippedItem, QaEntry, QaExportBatch, QaSessionExport, SessionHandle, SessionMeta,
    SessionProvenance, SessionScope,
};
use std::time::Instant;

/// Select sessions whose activity timestamp falls in `[from, to]` (epoch
/// milliseconds, inclusive — the app-wide timestamp unit) and distill them
/// into Q&A exports with provenance.
///
/// - Sessions missing both `last_active_at` and `created_at` are excluded.
/// - Individual load failures are recorded in `skipped`; the batch continues.
pub fn export_qa_sessions(
    registry: &ProviderRegistry,
    scope: &SessionScope,
    from: i64,
    to: i64,
    providers: Option<&[String]>,
) -> QaExportBatch {
    let start = Instant::now();
    log::debug!(
        "qa_export start scope={} from={} to={} providers={:?}",
        scope_label(scope),
        from,
        to,
        providers
    );

    let selected: Vec<SessionMeta> = scan_sessions_with_scope(registry, scope)
        .into_iter()
        .filter(|meta| in_provider_set(meta, providers))
        .filter(|meta| session_in_range(meta, from, to))
        .collect();

    let mut sessions = Vec::with_capacity(selected.len());
    let mut skipped = Vec::new();

    for meta in &selected {
        export_one(registry, meta, &mut sessions, &mut skipped);
    }

    log::debug!(
        "qa_export finish scope={} selected={} exported={} skipped={} elapsed_ms={}",
        scope_label(scope),
        selected.len(),
        sessions.len(),
        skipped.len(),
        start.elapsed().as_millis()
    );

    QaExportBatch { sessions, skipped }
}

/// Export an explicit, already-filtered session list ("export what you see"):
/// the UI adapter (folder/search/star/time filters) selects the sessions and
/// passes their `SessionMeta`; this core only distills and assembles
/// provenance. Selection logic stays out of the core by design.
pub fn export_qa_sessions_for_metas(
    registry: &ProviderRegistry,
    metas: &[SessionMeta],
) -> QaExportBatch {
    let start = Instant::now();
    log::debug!("qa_export_for_metas start count={}", metas.len());

    let mut sessions = Vec::with_capacity(metas.len());
    let mut skipped = Vec::new();

    for meta in metas {
        export_one(registry, meta, &mut sessions, &mut skipped);
    }

    log::debug!(
        "qa_export_for_metas finish selected={} exported={} skipped={} elapsed_ms={}",
        metas.len(),
        sessions.len(),
        skipped.len(),
        start.elapsed().as_millis()
    );

    QaExportBatch { sessions, skipped }
}

/// Load, distill, and append one session's export; record failures in
/// `skipped` without aborting the batch.
fn export_one(
    registry: &ProviderRegistry,
    meta: &SessionMeta,
    sessions: &mut Vec<QaSessionExport>,
    skipped: &mut Vec<ExportSkippedItem>,
) {
    let Some(handle) = handle_from_meta(meta) else {
        skipped.push(ExportSkippedItem {
            provider_id: meta.provider_id.clone(),
            session_id: meta.session_id.clone(),
            error: "session has no loadable locator".to_string(),
        });
        return;
    };
    match load_messages_for_handle(registry, &handle) {
        Ok(messages) => {
            let qa = extract_qa_entries(&messages);
            sessions.push(QaSessionExport {
                provenance: provenance_from_meta(meta),
                qa,
            });
        }
        Err(err) => {
            log::warn!(
                "qa_export skip provider={} session={} error={}",
                meta.provider_id,
                meta.session_id,
                err
            );
            skipped.push(ExportSkippedItem {
                provider_id: meta.provider_id.clone(),
                session_id: meta.session_id.clone(),
                error: err,
            });
        }
    }
}

/// Distill messages into materialized Q&A entries using merge semantics:
/// all same-turn assistant texts are joined (`"\n\n"`); tool calls, tool
/// results, and non-conversation roles are skipped; `ts` comes from the
/// question message.
pub fn extract_qa_entries(messages: &[super::types::SessionMessage]) -> Vec<QaEntry> {
    let mut entries = Vec::new();
    let mut pending_question: Option<(String, Option<i64>)> = None;
    let mut pending_answer: Vec<String> = Vec::new();

    let flush = |entries: &mut Vec<QaEntry>,
                 question: &mut Option<(String, Option<i64>)>,
                 answer: &mut Vec<String>| {
        if let Some((q, ts)) = question.take() {
            if !answer.is_empty() {
                entries.push(QaEntry {
                    question: q,
                    answer: answer.join("\n\n"),
                    ts,
                });
            }
        }
        answer.clear();
    };

    for message in messages {
        match message.role.to_lowercase().as_str() {
            "user" => {
                flush(&mut entries, &mut pending_question, &mut pending_answer);
                pending_question = Some((message.content.clone(), message.ts));
            }
            "assistant" => {
                if pending_question.is_some() && !message.content.trim().is_empty() {
                    pending_answer.push(message.content.clone());
                }
            }
            _ => {}
        }
    }
    flush(&mut entries, &mut pending_question, &mut pending_answer);

    entries
}

fn session_in_range(meta: &SessionMeta, from: i64, to: i64) -> bool {
    meta.last_active_at
        .or(meta.created_at)
        .is_some_and(|ts| ts >= from && ts <= to)
}

fn in_provider_set(meta: &SessionMeta, providers: Option<&[String]>) -> bool {
    match providers {
        None => true,
        Some(set) => set.iter().any(|p| p == &meta.provider_id),
    }
}

fn handle_from_meta(meta: &SessionMeta) -> Option<SessionHandle> {
    let locator = meta.locator.clone()?;
    Some(SessionHandle {
        provider_id: meta.provider_id.clone(),
        session_id: meta.session_id.clone(),
        locator,
    })
}

fn provenance_from_meta(meta: &SessionMeta) -> SessionProvenance {
    SessionProvenance {
        provider_id: meta.provider_id.clone(),
        session_id: meta.session_id.clone(),
        title: meta.title.clone(),
        project_dir: meta.project_dir.clone(),
        created_at: meta.created_at,
        last_active_at: meta.last_active_at,
        locator: meta.locator.clone(),
    }
}

fn scope_label(scope: &SessionScope) -> &'static str {
    match scope {
        SessionScope::Active => "active",
        SessionScope::Archived => "archived",
    }
}

// ─── Rendering ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QaExportFormat {
    Json,
    Jsonl,
    Markdown,
}

impl QaExportFormat {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.to_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "jsonl" | "ndjson" => Ok(Self::Jsonl),
            "markdown" | "md" => Ok(Self::Markdown),
            other => Err(format!(
                "Unknown export format: {other} (expected json, jsonl, or markdown)"
            )),
        }
    }
}

/// Render the batch plus export envelope (range, timestamp) into the
/// requested file format. `include_provenance: false` omits provenance
/// blocks/fields entirely (CLI `--no-metadata`); provenance is on by
/// default.
pub fn render_export(
    batch: &QaExportBatch,
    from: i64,
    to: i64,
    format: QaExportFormat,
    include_provenance: bool,
) -> Result<String, String> {
    let exported_at = chrono::Utc::now().timestamp();
    match format {
        QaExportFormat::Json => render_json(batch, from, to, exported_at, include_provenance),
        QaExportFormat::Jsonl => render_jsonl(batch, include_provenance),
        QaExportFormat::Markdown => {
            Ok(render_markdown(batch, from, to, exported_at, include_provenance))
        }
    }
}

/// NDJSON rendering: one session per line, so repeated runs can be appended
/// with `>>` and the file stays valid JSON Lines. Per-run envelope metadata
/// (range, timestamp, skips) is NOT embedded — skips go to stderr in the
/// CLI adapter; append-safety is the framing contract.
fn render_jsonl(batch: &QaExportBatch, include_provenance: bool) -> Result<String, String> {
    use std::fmt::Write as _;
    let mut out = String::new();
    for session in &batch.sessions {
        let line = if include_provenance {
            serde_json::to_string(session)
        } else {
            #[derive(serde::Serialize)]
            #[serde(rename_all = "camelCase")]
            struct BareSession<'a> {
                qa: &'a [QaEntry],
            }
            serde_json::to_string(&BareSession { qa: &session.qa })
        };
        let line = line.map_err(|e| format!("Failed to serialize export: {e}"))?;
        let _ = writeln!(out, "{line}");
    }
    Ok(out)
}

fn render_json(
    batch: &QaExportBatch,
    from: i64,
    to: i64,
    exported_at: i64,
    include_provenance: bool,
) -> Result<String, String> {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Envelope<'a> {
        exported_at: i64,
        range: (i64, i64),
        sessions: &'a [QaSessionExport],
        skipped: &'a [ExportSkippedItem],
    }
    let envelope = Envelope {
        exported_at,
        range: (from, to),
        sessions: &batch.sessions,
        skipped: &batch.skipped,
    };
    if include_provenance {
        return serde_json::to_string_pretty(&envelope)
            .map_err(|e| format!("Failed to serialize export: {e}"));
    }
    // Provenance-free view: sessions rendered as bare Q&A lists.
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct BareSession<'a> {
        qa: &'a [QaEntry],
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct BareEnvelope<'a> {
        exported_at: i64,
        range: (i64, i64),
        sessions: Vec<BareSession<'a>>,
        skipped: &'a [ExportSkippedItem],
    }
    let bare = BareEnvelope {
        exported_at,
        range: (from, to),
        sessions: batch
            .sessions
            .iter()
            .map(|s| BareSession { qa: &s.qa })
            .collect(),
        skipped: &batch.skipped,
    };
    serde_json::to_string_pretty(&bare).map_err(|e| format!("Failed to serialize export: {e}"))
}

fn render_markdown(
    batch: &QaExportBatch,
    from: i64,
    to: i64,
    exported_at: i64,
    include_provenance: bool,
) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# Q&A Session Export\n\n- Range: `{from}` – `{to}` (epoch ms)\n- Exported at: `{exported_at}`\n- Sessions: {}\n- Skipped: {}\n",
        batch.sessions.len(),
        batch.skipped.len()
    );

    for session in &batch.sessions {
        let title = session
            .provenance
            .title
            .as_deref()
            .unwrap_or(&session.provenance.session_id);
        let _ = writeln!(out, "## {title}\n");
        for entry in &session.qa {
            let _ = writeln!(out, "**Q:** {}\n", entry.question);
            let _ = writeln!(out, "**A:** {}\n", entry.answer);
        }
        if session.qa.is_empty() {
            let _ = writeln!(out, "_(no Q&A pairs)_\n");
        }
        if include_provenance {
            let p = &session.provenance;
            let _ = writeln!(out, "<details><summary>Provenance</summary>\n");
            let _ = writeln!(out, "- provider: `{}`", p.provider_id);
            let _ = writeln!(out, "- session: `{}`", p.session_id);
            if let Some(dir) = &p.project_dir {
                let _ = writeln!(out, "- project: `{dir}`");
            }
            if let Some(ts) = p.created_at {
                let _ = writeln!(out, "- createdAt: `{ts}`");
            }
            if let Some(ts) = p.last_active_at {
                let _ = writeln!(out, "- lastActiveAt: `{ts}`");
            }
            if let Some(locator) = &p.locator {
                let _ = writeln!(out, "- locator: `{}`", locator.detail_key_part());
            }
            let _ = writeln!(out, "\n</details>\n");
        }
    }
    out
}

/// Write the rendered export to `path`. Refuses to overwrite an existing
/// destination unless `overwrite` is explicitly set — the default refusal is
/// the adapter-independent safety rule (a CLI has no save dialog to confirm);
/// interactive adapters may pass `overwrite: true` when the native dialog has
/// already obtained the user's confirmation.
pub fn write_export_file(path: &Path, content: &str, overwrite: bool) -> Result<(), String> {
    if path.exists() && !overwrite {
        return Err(format!(
            "Destination file already exists: {} (choose another name)",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create export directory {}: {e}", parent.display()))?;
    }
    std::fs::write(path, content).map_err(|e| format!("Failed to write export file {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::super::types::{SessionLocator, SessionMessage};
    use super::*;

    fn message(role: &str, content: &str) -> SessionMessage {
        SessionMessage {
            role: role.to_string(),
            content: content.to_string(),
            ts: None,
            usage: None,
            cumulative_usage: None,
            tool_calls: None,
            tool_result: None,
        }
    }

    fn message_ts(role: &str, content: &str, ts: i64) -> SessionMessage {
        SessionMessage { ts: Some(ts), ..message(role, content) }
    }

    #[test]
    fn qa_entries_merge_same_turn_assistant_texts() {
        let msgs = vec![
            message("user", "u1"),
            message("assistant", "thinking"),
            message("assistant", "final answer"),
            message("user", "u2"),
            message("assistant", "a2"),
        ];
        let entries = extract_qa_entries(&msgs);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].question, "u1");
        assert_eq!(entries[0].answer, "thinking\n\nfinal answer");
        assert_eq!(entries[1].question, "u2");
        assert_eq!(entries[1].answer, "a2");
    }

    #[test]
    fn qa_entries_skip_tool_and_system_roles() {
        let msgs = vec![
            message("user", "u1"),
            message("system", "sys"),
            message("tool", "tool output"),
            message("assistant", "a1"),
        ];
        let entries = extract_qa_entries(&msgs);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].answer, "a1");
    }

    #[test]
    fn qa_entries_take_ts_from_question() {
        let msgs = vec![message_ts("user", "u1", 100), message_ts("assistant", "a1", 200)];
        let entries = extract_qa_entries(&msgs);

        assert_eq!(entries[0].ts, Some(100));
    }

    #[test]
    fn qa_entries_skip_empty_assistant_and_unanswered_users() {
        let msgs = vec![
            message("user", "u1"),
            message("assistant", "   "),
            message("user", "u2"), // u1 unanswered -> dropped
            message("assistant", "a2"),
        ];
        let entries = extract_qa_entries(&msgs);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].question, "u2");
    }

    #[test]
    fn qa_entries_emit_final_pair() {
        let msgs = vec![message("user", "u1"), message("assistant", "a1")];
        let entries = extract_qa_entries(&msgs);
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn format_parser_rejects_unknown() {
        assert!(QaExportFormat::parse("json").is_ok());
        assert!(QaExportFormat::parse("Markdown").is_ok());
        assert!(QaExportFormat::parse("xml").is_err());
    }

    #[test]
    fn render_json_envelope_roundtrips() {
        let batch = QaExportBatch {
            sessions: vec![QaSessionExport {
                provenance: SessionProvenance {
                    provider_id: "claude".to_string(),
                    session_id: "s1".to_string(),
                    title: Some("t".to_string()),
                    project_dir: None,
                    created_at: Some(1),
                    last_active_at: Some(2),
                    locator: Some(SessionLocator::File {
                        path: "/tmp/s1.jsonl".to_string(),
                    }),
                },
                qa: vec![QaEntry {
                    question: "q".to_string(),
                    answer: "a".to_string(),
                    ts: Some(1),
                }],
            }],
            skipped: vec![],
        };
        let json = render_export(&batch, 0, 10, QaExportFormat::Json, true).expect("render");
        let value: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(value["range"][0], 0);
        assert_eq!(value["sessions"][0]["provenance"]["sessionId"], "s1");
        assert_eq!(value["sessions"][0]["qa"][0]["question"], "q");
    }

    #[test]
    fn write_export_file_overwrite_semantics() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("export.json");
        write_export_file(&path, "{}", false).expect("first write");
        let err = write_export_file(&path, "{}", false).expect_err("should refuse overwrite");
        assert!(err.contains("already exists"), "unexpected: {err}");
        write_export_file(&path, "{\"v\":2}", true).expect("explicit overwrite allowed");
        assert_eq!(std::fs::read_to_string(&path).expect("reread"), "{\"v\":2}");
    }

    // ─── End-to-end over a real provider scan ───────────────────────────

    fn write_claude_session_with_ts(path: &std::path::Path, session_id: &str, ts: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create dir");
        std::fs::write(
            path,
            format!(
                "{{\"sessionId\":\"{session_id}\",\"cwd\":\"/tmp/project\",\"timestamp\":\"{ts}\"}}\n\
                 {{\"message\":{{\"role\":\"user\",\"content\":\"question\"}},\"timestamp\":\"{ts}\"}}\n\
                 {{\"message\":{{\"role\":\"assistant\",\"content\":\"part1\"}},\"timestamp\":\"{ts}\"}}\n\
                 {{\"message\":{{\"role\":\"assistant\",\"content\":\"part2\"}},\"timestamp\":\"{ts}\"}}\n",
            ),
        )
        .expect("write source");
    }

    #[test]
    fn export_for_metas_exports_explicit_list_and_skips_broken() {
        use crate::config::TEST_ENV_LOCK;
        let _guard = TEST_ENV_LOCK.lock().expect("lock");

        struct EnvVarGuard {
            key: &'static str,
            old_value: Option<std::ffi::OsString>,
        }
        impl Drop for EnvVarGuard {
            fn drop(&mut self) {
                if let Some(v) = &self.old_value {
                    std::env::set_var(self.key, v);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }

        let test_home = tempfile::tempdir().expect("tempdir");
        let old = std::env::var_os("SESSION_MANAGER_TEST_HOME");
        std::env::set_var("SESSION_MANAGER_TEST_HOME", test_home.path());
        let _guard_env = EnvVarGuard { key: "SESSION_MANAGER_TEST_HOME", old_value: old };

        let projects = test_home.path().join(".claude").join("projects").join("folder");
        let ts = "2026-09-09T10:00:00Z";
        write_claude_session_with_ts(&projects.join("picked.jsonl"), "picked", ts);

        let meta = |provider: &str, id: &str, path: Option<String>| SessionMeta {
            provider_id: provider.to_string(),
            session_id: id.to_string(),
            title: None,
            summary: None,
            project_dir: None,
            created_at: Some(1),
            last_active_at: Some(2),
            source_path: path.clone(),
            locator: path.map(|p| SessionLocator::File { path: p }),
            resume_command: None,
            forked_from_id: None,
        };

        let registry = super::super::build_provider_registry();
        let batch = export_qa_sessions_for_metas(
            &registry,
            &[
                meta("claude", "picked", Some(projects.join("picked.jsonl").to_string_lossy().into_owned())),
                meta("claude", "missing-file", Some(projects.join("gone.jsonl").to_string_lossy().into_owned())),
            ],
        );

        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].provenance.session_id, "picked");
        assert_eq!(batch.sessions[0].qa.len(), 1);
        assert_eq!(batch.skipped.len(), 1);
        assert_eq!(batch.skipped[0].session_id, "missing-file");
    }

    #[test]
    fn export_qa_sessions_filters_by_time_and_merges_answers() {
        use crate::config::TEST_ENV_LOCK;
        let _guard = TEST_ENV_LOCK.lock().expect("lock");

        struct EnvVarGuard {
            key: &'static str,
            old_value: Option<std::ffi::OsString>,
        }
        impl Drop for EnvVarGuard {
            fn drop(&mut self) {
                if let Some(v) = &self.old_value {
                    std::env::set_var(self.key, v);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }

        let test_home = tempfile::tempdir().expect("tempdir");
        let old = std::env::var_os("SESSION_MANAGER_TEST_HOME");
        std::env::set_var("SESSION_MANAGER_TEST_HOME", test_home.path());
        let _guard_env = EnvVarGuard { key: "SESSION_MANAGER_TEST_HOME", old_value: old };

        let projects = test_home.path().join(".claude").join("projects").join("folder");
        let ts = "2026-09-09T10:00:00Z";
        let in_range_ms = chrono::DateTime::parse_from_rfc3339(ts).expect("parse ts").timestamp_millis();
        write_claude_session_with_ts(&projects.join("in-range.jsonl"), "in-range", ts);
        write_claude_session_with_ts(&projects.join("out-of-range.jsonl"), "out-of-range", "2026-08-01T10:00:00Z");

        let registry = super::super::build_provider_registry();
        let batch = export_qa_sessions(
            &registry,
            &SessionScope::Active,
            in_range_ms - 3_600_000,
            in_range_ms + 3_600_000,
            None,
        );

        assert_eq!(batch.sessions.len(), 1);
        assert_eq!(batch.sessions[0].provenance.session_id, "in-range");
        assert_eq!(batch.sessions[0].qa.len(), 1);
        assert_eq!(batch.sessions[0].qa[0].answer, "part1\n\npart2");
        assert!(batch.skipped.is_empty());

        // Provider filter excludes everything.
        let empty = export_qa_sessions(
            &registry,
            &SessionScope::Active,
            in_range_ms - 3_600_000,
            in_range_ms + 3_600_000,
            Some(&["qoder".to_string()]),
        );
        assert!(empty.sessions.is_empty());
    }
}
