//! Thin IPC adapter for the settings core. All logic lives in
//! `session_manager::settings`; these handlers only delegate and translate.
//! The `settings-changed` event is emitted HERE, not inside the manager,
//! which stays Tauri-free for the CLI adapter.

use std::sync::Arc;

use tauri::Emitter;

use crate::session_manager::providers::ProviderRegistry;
use crate::session_manager::settings::{SettingsManager, SettingValue, SettingsSnapshot};

/// Read-only provider id listing for the settings UI's source editor. Reuses
/// the same registry that backs the `agents` CLI subcommand — one source of
/// truth for "which providers exist" (explicit-provider model).
#[tauri::command]
pub fn list_providers(registry: tauri::State<'_, Arc<ProviderRegistry>>) -> Vec<String> {
    registry.ids().cloned().collect()
}

#[tauri::command]
pub fn get_settings(manager: tauri::State<'_, SettingsManager>) -> SettingsSnapshot {
    manager.get()
}

#[tauri::command]
pub fn set_setting_value(
    app: tauri::AppHandle,
    manager: tauri::State<'_, SettingsManager>,
    key: String,
    value: SettingValue,
) -> Result<(), String> {
    let keys = manager.set_value(&key, value)?;
    app.emit("settings-changed", serde_json::json!({ "keys": keys }))
        .map_err(|e| format!("Failed to emit settings-changed: {e}"))?;
    Ok(())
}
