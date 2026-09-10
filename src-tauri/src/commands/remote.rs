//! Thin IPC adapter for the SSH add-source UI flow (ADR 0010 / ADR
//! 0008 修订 1). All logic lives in `session_manager::remote` — these
//! handlers only delegate, translate, and enforce the UI-facing
//! timeout. exec/SFTP calls never happen here (remote-module-only
//! discipline); `test_ssh_source` merely drives the remote layer's
//! public pool + scan API.

use std::sync::Arc;
use std::time::Duration;

use crate::session_manager::providers::ProviderRegistry;
use crate::session_manager::remote::{self, RemoteScanState, SshAliasInfo, SshTestResult};
use crate::session_manager::settings::SshSource;

/// Upper bound for one test connection, so a black-holed host cannot
/// hang the settings UI (connect + auth + one quick scan normally
/// completes in well under a second on a reachable LAN host).
const TEST_TIMEOUT: Duration = Duration::from_secs(15);

/// List every selectable Host alias from the user's `~/.ssh/config`
/// (Include honored) for the "Add SSH source" picker. A missing/empty
/// config returns an EMPTY Vec — not an error: the frontend switches
/// to its empty-state guide (config path + example block + manual
/// fallback form) on exactly that signal. ProxyJump blocks are listed
/// with `supported: false` so the UI can grey them out.
#[tauri::command]
pub async fn list_ssh_aliases() -> Result<Vec<SshAliasInfo>, String> {
    remote::list_aliases().map_err(|e| e.to_string())
}

/// The absolute `~/.ssh/config` path for the add-flow's empty-state
/// guide (copyable). Split from `list_ssh_aliases` so the picker's
/// payload stays the fixed `Vec<SshAliasInfo>` wire shape while the
/// guide still gets an OS-accurate path.
#[tauri::command]
pub fn get_ssh_config_path() -> Result<String, String> {
    remote::ssh_config_path()
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}

/// Test a DRAFT ssh source before the user commits it: connect +
/// authenticate + known_hosts gate (sshConfig aliases resolve through
/// `~/.ssh/config` first), then one quick Active-scope scan so the UI
/// can report "Connected — N sessions found". Failures come back as
/// `ok: false` with the actionable `RemoteError` display text
/// (ProxyJump / unknown host / auth each have their own remedy); a
/// black-holed host is bounded by [`TEST_TIMEOUT`].
/// Elapsed-timeout fallback: surfaced as the SAME result shape as a
/// normal failure (ok=false + actionable text), not a command-level
/// Err string — the UI renders it in the test-outcome slot.
fn timeout_fallback() -> SshTestResult {
    SshTestResult::fail(format!(
        "test connection timed out after {}s — the host may be unreachable",
        TEST_TIMEOUT.as_secs()
    ))
}

#[tauri::command]
pub async fn test_ssh_source(
    registry: tauri::State<'_, Arc<ProviderRegistry>>,
    remote_state: tauri::State<'_, RemoteScanState>,
    request: SshSource,
) -> Result<SshTestResult, String> {
    match tokio::time::timeout(
        TEST_TIMEOUT,
        remote::test_source(&registry, &remote_state.pool, &request),
    )
    .await
    {
        Ok(result) => Ok(result),
        Err(_) => Ok(timeout_fallback()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The IPC timeout wrapper must map an elapsed deadline onto the
    /// SAME wire shape as a normal failure (ok=false + actionable
    /// text), not a command-level Err string.
    #[tokio::test]
    async fn timeout_maps_onto_the_failure_result_shape() {
        let never = std::future::pending::<SshTestResult>();
        // A zero timeout guarantees elapse without waiting; the match
        // mirrors the command's own timeout handling.
        let result = match tokio::time::timeout(Duration::from_secs(0), never).await {
            Ok(result) => result,
            Err(_) => timeout_fallback(),
        };
        assert!(!result.ok);
        assert!(result.error.as_deref().expect("error").contains("timed out"));
        assert_eq!(result.session_count, None);
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json.get("sessionCount"), None);
    }
}
