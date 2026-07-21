pub mod fork_tree;
pub mod session_manager;

pub(crate) fn default_scope() -> String {
    "active".to_string()
}

/// Run a blocking operation on the Tauri async runtime.
/// Clones the Arc from the Tauri state and spawns the closure on the blocking thread pool.
/// `$reg` is the local variable name for the cloned Arc, available inside `$fn`.
macro_rules! run_blocking {
    ($registry:expr, $reg:ident, $fn:expr) => {{
        let $reg = ::std::sync::Arc::clone(&$registry);
        tauri::async_runtime::spawn_blocking(move || $fn)
            .await
            .map_err(|e| format!("Background task failed: {e}"))?
    }};
}

pub(crate) use run_blocking;
