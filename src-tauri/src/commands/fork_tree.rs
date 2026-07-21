use std::sync::Arc;

use crate::fork_tree;
use crate::session_manager;
use crate::session_manager::providers::ProviderRegistry;

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkTreeOptions {
    #[serde(default = "super::default_scope")]
    pub scope: String,
    #[serde(default)]
    pub project_dir: Option<String>,
}

#[tauri::command]
pub async fn compute_fork_tree(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    options: Option<ForkTreeOptions>,
) -> Result<fork_tree::ForkTreeResult, String> {
    let opts = options.unwrap_or_else(|| ForkTreeOptions {
        scope: super::default_scope(),
        project_dir: None,
    });
    let session_scope = match opts.scope.as_str() {
        "archived" => session_manager::SessionScope::Archived,
        _ => session_manager::SessionScope::Active,
    };
    let reg = Arc::clone(&registry);

    tauri::async_runtime::spawn_blocking(move || {
        fork_tree::compute_fork_tree(&reg, &session_scope, opts.project_dir.as_deref())
    })
    .await
    .map_err(|e| format!("Failed to compute fork tree: {e}"))?
}

#[tauri::command]
pub async fn get_fork_tree() -> Result<fork_tree::ForkTreeResult, String> {
    tauri::async_runtime::spawn_blocking(move || fork_tree::get_fork_tree())
        .await
        .map_err(|e| format!("Failed to get fork tree from cache: {e}"))?
}
