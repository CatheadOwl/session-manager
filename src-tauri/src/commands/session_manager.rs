#![allow(non_snake_case)]

use std::sync::Arc;

use super::run_blocking;

use serde::Deserialize;

use crate::session_manager;
use crate::session_manager::metadata::MetadataManager;
use crate::session_manager::providers::ProviderRegistry;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsOptions {
    #[serde(default = "super::default_scope")]
    pub scope: String,
}

#[tauri::command]
pub async fn list_sessions(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    options: Option<ListSessionsOptions>,
) -> Result<Vec<session_manager::SessionMeta>, String> {
    let scope = options
        .map(|o| o.scope)
        .unwrap_or_else(super::default_scope);
    let session_scope = match scope.as_str() {
        "archived" => session_manager::SessionScope::Archived,
        _ => session_manager::SessionScope::Active,
    };
    Ok(run_blocking!(
        registry,
        reg,
        session_manager::scan_sessions_with_scope(&reg, &session_scope)
    ))
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
    options: ExportQaSessionsOptions,
) -> Result<session_manager::ExportOutcome, String> {
    let session_scope = match options.scope.as_str() {
        "archived" => session_manager::SessionScope::Archived,
        _ => session_manager::SessionScope::Active,
    };
    let format = session_manager::QaExportFormat::parse(&options.format)?;

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
