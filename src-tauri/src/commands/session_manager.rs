#![allow(non_snake_case)]

use std::sync::Arc;

use super::run_blocking;

use serde::Deserialize;

use crate::session_manager;
use crate::session_manager::metadata::MetadataManager;
use crate::session_manager::providers::ProviderRegistry;
use crate::session_manager::remote::{RemoteScanState, resolve_remote_to_local};
use crate::session_manager::settings::{SettingsManager, SshSource, SourceEntry};

/// Extract the enabled ssh source entries up front so blocking closures
/// can own them without borrowing the settings state (Send + Tauri-free
/// downstream code).
fn enabled_ssh_sources(settings: &SettingsManager) -> Vec<SshSource> {
    settings
        .enabled_sources()
        .into_iter()
        .filter_map(|e| match e {
            SourceEntry::Ssh(s) => Some(s),
            _ => None,
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsOptions {
    #[serde(default = "super::default_scope")]
    pub scope: String,
}

/// List sessions: local scan + remote (SSH) sources merged.
///
/// Remote line (phase 3): after the local
/// `scan_sessions_with_scope`, every enabled ssh source from the
/// settings overlay is scanned over the batch channel and appended.
/// The remote scan honors the SAME scope as the local scan — its roots
/// are derived per scope from each provider's `roots()` (active root
/// for Active, archived root for Archived), so remote sources enrich
/// both scopes instead of active-only.
#[tauri::command]
pub async fn list_sessions(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    settings: tauri::State<'_, SettingsManager>,
    remote: tauri::State<'_, RemoteScanState>,
    options: Option<ListSessionsOptions>,
) -> Result<Vec<session_manager::SessionMeta>, String> {
    let scope = options
        .map(|o| o.scope)
        .unwrap_or_else(super::default_scope);
    let session_scope = match scope.as_str() {
        "archived" => session_manager::SessionScope::Archived,
        _ => session_manager::SessionScope::Active,
    };
    // Read the settings sources overlay before entering the blocking task so
    // the closure stays Send and the scan core stays Tauri-free.
    let extra_sources = settings.enabled_sources();
    // Extract the ssh entries up front so the blocking closure can own
    // the whole overlay without borrowing it back.
    let ssh_sources: Vec<_> = extra_sources
        .iter()
        .filter_map(|e| match e {
            SourceEntry::Ssh(s) => Some(s.clone()),
            _ => None,
        })
        .collect();
    let mut sessions = run_blocking!(
        registry,
        reg,
        session_manager::scan_sessions_with_scope(&reg, &session_scope, &extra_sources)
    );
    for source in &ssh_sources {
        let session = match remote.pool.get(source).await {
            Ok(session) => session,
            Err(err) => {
                // First-connect failure: cached list (empty on the
                // very first run) + warn — never block local listing.
                log::warn!(
                    "remote source `{}` unreachable ({err}) — serving cached sessions",
                    source.id
                );
                sessions.extend(remote.cached_sessions(&source.id));
                continue;
            }
        };
        let result = remote
            .scan_source(&registry, session, source, &session_scope)
            .await;
        sessions.extend(result.sessions);
    }
    // Global ordering across local + remote (same comparator as the
    // local scan core). Applied whenever any remote results exist.
    sessions.sort_by(|a, b| {
        let a_ts = a.last_active_at.or(a.created_at).unwrap_or(0);
        let b_ts = b.last_active_at.or(b.created_at).unwrap_or(0);
        b_ts.cmp(&a_ts)
    });
    Ok(sessions)
}

/// Load a session's messages. Remote (SSH) locators are bridged FIRST:
/// `resolve_remote_to_local` fetches the file into the transient cache
/// (cache hit = zero network) and hands the provider a File-locator handle
/// pointing at the local cache copy — the remote
/// source's only content-read path.
#[tauri::command]
pub async fn get_session_messages(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    settings: tauri::State<'_, SettingsManager>,
    remote: tauri::State<'_, RemoteScanState>,
    providerId: String,
    sourcePath: Option<String>,
    sessionId: Option<String>,
    locator: Option<session_manager::SessionLocator>,
) -> Result<Vec<session_manager::SessionMessage>, String> {
    let request = session_manager::SessionHandleRequest {
        provider_id: providerId,
        session_id: sessionId.unwrap_or_default(),
        source_path: sourcePath,
        locator,
    };
    let handle = request.into_handle()?;
    let ssh_sources = enabled_ssh_sources(&settings);
    let handle = match resolve_remote_to_local(&ssh_sources, &remote.pool, &handle).await? {
        Some(bridged) => bridged,
        None => handle,
    };
    run_blocking!(
        registry,
        reg,
        session_manager::load_messages_for_handle(&reg, &handle)
    )
}

/// Load a session's detail (messages + Q&A pairs + raw-content fallback).
/// Remote locators use the same bridge as `get_session_messages`: the
/// raw-content fallback also resolves through the File-locator cache copy,
/// so file-backed providers need no Remote awareness.
#[tauri::command]
pub async fn get_session_detail(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    settings: tauri::State<'_, SettingsManager>,
    remote: tauri::State<'_, RemoteScanState>,
    providerId: String,
    sourcePath: Option<String>,
    sessionId: Option<String>,
    locator: Option<session_manager::SessionLocator>,
) -> Result<session_manager::SessionDetail, String> {
    let request = session_manager::SessionHandleRequest {
        provider_id: providerId,
        session_id: sessionId.unwrap_or_default(),
        source_path: sourcePath,
        locator,
    };
    let handle = request.into_handle()?;
    let ssh_sources = enabled_ssh_sources(&settings);
    let handle = match resolve_remote_to_local(&ssh_sources, &remote.pool, &handle).await? {
        Some(bridged) => bridged,
        None => handle,
    };
    run_blocking!(
        registry,
        reg,
        session_manager::load_session_detail_for_handle(&reg, &handle)
    )
}

#[tauri::command]
pub async fn delete_session(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    providerId: String,
    sessionId: String,
    sourcePath: Option<String>,
    locator: Option<session_manager::SessionLocator>,
) -> Result<bool, String> {
    let request = session_manager::SessionHandleRequest {
        provider_id: providerId,
        session_id: sessionId,
        source_path: sourcePath,
        locator,
    };
    let handle = request.into_handle()?;
    run_blocking!(
        registry,
        reg,
        session_manager::delete_session_for_handle(&reg, &handle)
    )
}

#[tauri::command]
pub async fn delete_sessions(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    items: Vec<session_manager::DeleteSessionRequest>,
) -> Result<Vec<session_manager::DeleteSessionOutcome>, String> {
    Ok(run_blocking!(
        registry,
        reg,
        session_manager::delete_sessions(&reg, &items)
    ))
}

#[tauri::command]
pub async fn archive_sessions(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    items: Vec<session_manager::DeleteSessionRequest>,
) -> Result<Vec<session_manager::DeleteSessionOutcome>, String> {
    Ok(run_blocking!(
        registry,
        reg,
        session_manager::archive_sessions(&reg, &items)
    ))
}

#[tauri::command]
pub async fn restore_sessions(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    items: Vec<session_manager::DeleteSessionRequest>,
) -> Result<Vec<session_manager::DeleteSessionOutcome>, String> {
    Ok(run_blocking!(
        registry,
        reg,
        session_manager::restore_sessions(&reg, &items)
    ))
}

#[tauri::command]
pub async fn archive_session(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    providerId: String,
    sessionId: String,
    sourcePath: Option<String>,
    locator: Option<session_manager::SessionLocator>,
) -> Result<bool, String> {
    let request = session_manager::SessionHandleRequest {
        provider_id: providerId,
        session_id: sessionId,
        source_path: sourcePath,
        locator,
    };
    let handle = request.into_handle()?;
    run_blocking!(
        registry,
        reg,
        session_manager::archive_session_for_handle(&reg, &handle)
    )
}

#[tauri::command]
pub async fn restore_session(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    providerId: String,
    sessionId: String,
    sourcePath: Option<String>,
    locator: Option<session_manager::SessionLocator>,
) -> Result<bool, String> {
    let request = session_manager::SessionHandleRequest {
        provider_id: providerId,
        session_id: sessionId,
        source_path: sourcePath,
        locator,
    };
    let handle = request.into_handle()?;
    run_blocking!(
        registry,
        reg,
        session_manager::restore_session_for_handle(&reg, &handle)
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportQaSessionsOptions {
    #[serde(default = "super::default_scope")]
    pub scope: String,
    /// Inclusive epoch-milliseconds window (app-wide timestamp unit).
    pub from: i64,
    pub to: i64,
    #[serde(default)]
    pub providers: Option<Vec<String>>,
    /// Absolute destination file path chosen via the native save dialog.
    pub dest_path: String,
    #[serde(default = "default_export_format")]
    pub format: String,
    /// Whether writing may overwrite an existing destination file. The UI
    /// adapter passes `true` because the native save dialog has already
    /// asked the user to confirm replacement; non-interactive adapters
    /// should keep the safe default (refuse).
    #[serde(default)]
    pub overwrite: bool,
    /// Explicit pre-filtered session list ("export what you see": the UI has
    /// already applied folder/search/star/time filters, and narrowed it to
    /// the checked sessions when selection mode is on). When absent, the
    /// core falls back to scanning by the time window (future CLI path).
    #[serde(default)]
    pub sessions: Option<Vec<session_manager::SessionMeta>>,
}

fn default_export_format() -> String {
    "json".to_string()
}

/// Bridge the Remote-locator metas of an export selection into local cache
/// copies before the blocking distill core runs. Each Remote meta is fetched via
/// `resolve_remote_to_local` — the same bridge as `get_session_messages`
/// (mtime+size-gated cache exit: re-exports of the same sessions
/// are free) — and its local path is returned as a load override keyed by
/// the meta's index in the returned list. A meta whose fetch fails (source
/// disabled/unreachable, transfer error) is pre-skipped and REMOVED from
/// the list so the core records exactly one skip per item, never two.
pub(crate) async fn bridge_remote_export_metas(
    ssh_sources: &[SshSource],
    pool: &crate::session_manager::remote::RemoteSessionPool,
    sessions: Vec<session_manager::SessionMeta>,
) -> (
    Vec<session_manager::SessionMeta>,
    std::collections::HashMap<usize, std::path::PathBuf>,
    Vec<session_manager::ExportSkippedItem>,
) {
    let mut metas = Vec::with_capacity(sessions.len());
    let mut overrides = std::collections::HashMap::new();
    let mut skipped = Vec::new();
    for meta in sessions {
        let remote_locator = match &meta.locator {
            Some(loc @ session_manager::SessionLocator::Remote { .. }) => loc.clone(),
            _ => {
                metas.push(meta);
                continue;
            }
        };
        let handle = session_manager::SessionHandle {
            provider_id: meta.provider_id.clone(),
            session_id: meta.session_id.clone(),
            locator: remote_locator,
        };
        match resolve_remote_to_local(ssh_sources, pool, &handle).await {
            Ok(Some(bridged)) => match &bridged.locator {
                session_manager::SessionLocator::File { path } => {
                    overrides.insert(metas.len(), std::path::PathBuf::from(path));
                    metas.push(meta);
                }
                // Unreachable by construction: the bridge resolves to a
                // File locator backed by the transient cache.
                _ => skipped.push(remote_fetch_skip(&meta, "remote bridge returned no local path")),
            },
            Ok(None) => skipped.push(remote_fetch_skip(&meta, "remote bridge returned no local path")),
            Err(err) => skipped.push(remote_fetch_skip(&meta, err)),
        }
    }
    (metas, overrides, skipped)
}

fn remote_fetch_skip(
    meta: &session_manager::SessionMeta,
    error: impl Into<String>,
) -> session_manager::ExportSkippedItem {
    session_manager::ExportSkippedItem {
        provider_id: meta.provider_id.clone(),
        session_id: meta.session_id.clone(),
        error: error.into(),
    }
}

/// Adapter for the export core: translates parameters, delegates all logic
/// to `session_manager::export_qa_sessions`, renders, and writes the file.
/// Remote-locator metas in an explicit selection are bridged first (see
/// [`bridge_remote_export_metas`]); the None branch (time-window scan, the
/// future CLI path) stays local-only — the scan's extra sources are local
/// mirrors by definition (each local extra source is an alternate home).
#[tauri::command]
pub async fn export_qa_sessions(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    settings: tauri::State<'_, SettingsManager>,
    remote: tauri::State<'_, RemoteScanState>,
    options: ExportQaSessionsOptions,
) -> Result<session_manager::ExportOutcome, String> {
    let session_scope = match options.scope.as_str() {
        "archived" => session_manager::SessionScope::Archived,
        _ => session_manager::SessionScope::Active,
    };
    let format = session_manager::QaExportFormat::parse(&options.format)?;

    let extra_sources = settings.enabled_sources();
    let batch = if let Some(sessions) = options.sessions {
        let (metas, overrides, pre_skipped) =
            bridge_remote_export_metas(&enabled_ssh_sources(&settings), &remote.pool, sessions)
                .await;
        let mut batch = run_blocking!(
            registry,
            reg,
            session_manager::export_qa_sessions_for_metas_with_overrides(&reg, &metas, &overrides)
        );
        batch.skipped.extend(pre_skipped);
        batch
    } else {
        run_blocking!(
            registry,
            reg,
            session_manager::export_qa_sessions(
                &reg,
                &session_scope,
                options.from,
                options.to,
                options.providers.as_deref(),
                &extra_sources,
            )
        )
    };

    let content = session_manager::render_export(&batch, options.from, options.to, format, true)?;
    let dest = std::path::PathBuf::from(&options.dest_path);
    session_manager::write_export_file(&dest, &content, options.overwrite)?;

    Ok(session_manager::ExportOutcome {
        count: batch.sessions.len(),
        skipped: batch.skipped,
        dest_path: options.dest_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The IPC wire format is camelCase; verify the option bundle round-trips
    /// with the fields the frontend actually sends, and that optional fields
    /// (`sessions`, `providers`, `overwrite`) default safely.
    #[test]
    fn export_options_deserialize_camel_case_wire() {
        let full: ExportQaSessionsOptions = serde_json::from_str(
            r#"{
                "scope": "archived",
                "from": 1000,
                "to": 2000,
                "providers": ["claude"],
                "sessions": [
                    {
                        "providerId": "claude",
                        "sessionId": "s1",
                        "locator": { "kind": "file", "path": "/tmp/s1.jsonl" }
                    }
                ],
                "destPath": "/tmp/out.json",
                "format": "markdown",
                "overwrite": true
            }"#,
        )
        .expect("full payload");

        assert_eq!(full.scope, "archived");
        assert_eq!(full.from, 1000);
        assert_eq!(full.providers.as_deref(), Some(&["claude".to_string()][..]));
        let sessions = full.sessions.expect("sessions present");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "s1");
        assert_eq!(full.dest_path, "/tmp/out.json");
        assert_eq!(full.format, "markdown");
        assert!(full.overwrite);

        let minimal: ExportQaSessionsOptions = serde_json::from_str(
            r#"{ "from": 0, "to": 1, "destPath": "/tmp/out.json" }"#,
        )
        .expect("minimal payload");
        assert_eq!(minimal.scope, "active");
        assert_eq!(minimal.format, "json");
        assert!(minimal.providers.is_none());
        assert!(minimal.sessions.is_none());
        assert!(!minimal.overwrite);
    }
}

#[tauri::command]
pub async fn get_app_metadata(
    manager: tauri::State<'_, MetadataManager>,
) -> Result<session_manager::metadata::MetadataStore, String> {
    Ok(manager.get_metadata())
}

#[tauri::command]
pub async fn set_session_starred(
    manager: tauri::State<'_, MetadataManager>,
    sessionKey: String,
    starred: bool,
) -> Result<(), String> {
    manager.set_session_starred(&sessionKey, starred)
}

#[tauri::command]
pub async fn set_pinned_folders(
    manager: tauri::State<'_, MetadataManager>,
    folders: Vec<String>,
) -> Result<(), String> {
    manager.set_pinned_folders(folders)
}
