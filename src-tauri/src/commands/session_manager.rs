#![allow(non_snake_case)]

use std::sync::Arc;

use super::run_blocking;

use serde::Deserialize;
use tauri::Emitter;

use crate::session_manager;
use crate::session_manager::metadata::MetadataManager;
use crate::session_manager::providers::ProviderRegistry;
use crate::session_manager::remote::RemoteScanState;
use crate::session_manager::settings::{ProviderHintHeal, SettingsManager, SourceEntry};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsOptions {
    #[serde(default = "super::default_scope")]
    pub scope: String,
}

/// List sessions: local scan + remote (SSH) sources merged.
///
/// Remote line (ADR 0007/0008 phase 3): after the local
/// `scan_sessions_with_scope`, every enabled ssh source from the
/// settings overlay is scanned over the batch channel and appended.
/// This command is ALSO the auto-heal execution point (ADR 0008 §1a):
/// the scan core only decides (`RemoteSourceResult::heal`), while the
/// heal write + `settings-changed` event happen here — the only layer
/// that may emit events. The CLI adapter never calls heal.
#[tauri::command]
pub async fn list_sessions(
    app: tauri::AppHandle,
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
    let is_active = matches!(&session_scope, session_manager::SessionScope::Active);
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
    // Remote sources only enrich the active scope (a remote source has
    // no archive root; same rule as the local overlay).
    if is_active {
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
            let result = remote.scan_source(&registry, session, source).await;
            // Auto-heal execution (ADR 0008 §1a): Applied → persist done
            // inside SettingsManager; the event fires here. The sessions
            // returned by THIS scan stay valid — the hint takes effect on
            // the next scan.
            if let Some((source_id, hint)) = &result.heal {
                match settings.heal_provider_hint(source_id, hint) {
                    Ok(ProviderHintHeal::Applied) => {
                        app.emit(
                            "settings-changed",
                            serde_json::json!({ "keys": ["sources"] }),
                        )
                        .map_err(|e| format!("Failed to emit settings-changed: {e}"))?;
                        log::info!(
                            "remote scan: healed providerHint of source `{source_id}` to `{hint}`"
                        );
                    }
                    Ok(ProviderHintHeal::SkippedComments) => {
                        log::warn!(
                            "remote scan: provider hint heal for `{source_id}` skipped — \
                             settings file contains comments (ADR 0008 §1a)"
                        );
                    }
                    Ok(ProviderHintHeal::AlreadySet) => {}
                    Err(err) => {
                        log::warn!(
                            "remote scan: provider hint heal for `{source_id}` failed: {err}"
                        );
                    }
                }
            }
            sessions.extend(result.sessions);
        }
        // Global ordering across local + remote (same comparator as the
        // local scan core).
        sessions.sort_by(|a, b| {
            let a_ts = a.last_active_at.or(a.created_at).unwrap_or(0);
            let b_ts = b.last_active_at.or(b.created_at).unwrap_or(0);
            b_ts.cmp(&a_ts)
        });
    }
    Ok(sessions)
}

#[tauri::command]
pub async fn get_session_messages(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
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
    run_blocking!(
        registry,
        reg,
        session_manager::load_messages_for_handle(&reg, &handle)
    )
}

#[tauri::command]
pub async fn get_session_detail(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
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
    /// already applied folder/search/star/time filters). When absent, the
    /// core falls back to scanning by the time window (future CLI path).
    #[serde(default)]
    pub sessions: Option<Vec<session_manager::SessionMeta>>,
}

fn default_export_format() -> String {
    "json".to_string()
}

/// Adapter for the export core: translates parameters, delegates all logic
/// to `session_manager::export_qa_sessions`, renders, and writes the file.
#[tauri::command]
pub async fn export_qa_sessions(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    settings: tauri::State<'_, SettingsManager>,
    options: ExportQaSessionsOptions,
) -> Result<session_manager::ExportOutcome, String> {
    let session_scope = match options.scope.as_str() {
        "archived" => session_manager::SessionScope::Archived,
        _ => session_manager::SessionScope::Active,
    };
    let format = session_manager::QaExportFormat::parse(&options.format)?;

    let extra_sources = settings.enabled_sources();
    let batch = run_blocking!(
        registry,
        reg,
        match options.sessions {
            Some(ref sessions) => {
                session_manager::export_qa_sessions_for_metas(&reg, sessions)
            }
            None => session_manager::export_qa_sessions(
                &reg,
                &session_scope,
                options.from,
                options.to,
                options.providers.as_deref(),
                &extra_sources,
            ),
        }
    );

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
