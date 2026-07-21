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
    sourcePath: String,
) -> Result<Vec<session_manager::SessionMessage>, String> {
    run_blocking!(
        registry,
        reg,
        session_manager::load_messages(&reg, &providerId, &sourcePath)
    )
}

#[tauri::command]
pub async fn get_session_detail(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    providerId: String,
    sourcePath: String,
) -> Result<session_manager::SessionDetail, String> {
    run_blocking!(
        registry,
        reg,
        session_manager::load_session_detail(&reg, &providerId, &sourcePath)
    )
}

#[tauri::command]
pub async fn delete_session(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    providerId: String,
    sessionId: String,
    sourcePath: String,
) -> Result<bool, String> {
    run_blocking!(
        registry,
        reg,
        session_manager::delete_session(&reg, &providerId, &sessionId, &sourcePath)
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
    sourcePath: String,
) -> Result<bool, String> {
    run_blocking!(
        registry,
        reg,
        session_manager::archive_session(&reg, &providerId, &sessionId, &sourcePath)
    )
}

#[tauri::command]
pub async fn restore_session(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    providerId: String,
    sessionId: String,
    sourcePath: String,
) -> Result<bool, String> {
    run_blocking!(
        registry,
        reg,
        session_manager::restore_session(&reg, &providerId, &sessionId, &sourcePath)
    )
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
