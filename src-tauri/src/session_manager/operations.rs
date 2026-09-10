use std::path::{Path, PathBuf};

use super::providers::ProviderRegistry;
use super::types::{DeleteSessionOutcome, DeleteSessionRequest, SessionHandle};

const DB_OPERATION_UNSUPPORTED: &str =
    "Database-backed sessions are read-only and do not support this operation";

#[allow(dead_code)]
#[deprecated(note = "use delete_session_for_handle instead")]
pub fn delete_session(
    registry: &ProviderRegistry,
    provider_id: &str,
    session_id: &str,
    source_path: &str,
) -> Result<bool, String> {
    let handle = SessionHandle {
        provider_id: provider_id.to_string(),
        session_id: session_id.to_string(),
        locator: super::types::SessionLocator::File {
            path: source_path.to_string(),
        },
    };
    delete_session_for_handle(registry, &handle)
}

pub fn delete_session_for_handle(
    registry: &ProviderRegistry,
    handle: &SessionHandle,
) -> Result<bool, String> {
    let source_path = handle
        .file_path()
        .map_err(|_| DB_OPERATION_UNSUPPORTED.to_string())?;
    let provider = registry.get(&handle.provider_id)?;
    let roots = provider.roots();
    delete_session_with_roots(
        registry,
        &handle.provider_id,
        &handle.session_id,
        Path::new(source_path),
        &roots,
    )
}

pub fn delete_sessions(
    registry: &ProviderRegistry,
    requests: &[DeleteSessionRequest],
) -> Vec<DeleteSessionOutcome> {
    collect_session_outcomes(requests, "Session was not deleted", |request| {
        delete_session_for_handle(registry, &request.to_handle())
    })
}

pub fn archive_sessions(
    registry: &ProviderRegistry,
    requests: &[DeleteSessionRequest],
) -> Vec<DeleteSessionOutcome> {
    collect_session_outcomes(requests, "Session was not archived", |request| {
        archive_session_for_handle(registry, &request.to_handle())
    })
}

pub fn restore_sessions(
    registry: &ProviderRegistry,
    requests: &[DeleteSessionRequest],
) -> Vec<DeleteSessionOutcome> {
    collect_session_outcomes(requests, "Session was not restored", |request| {
        restore_session_for_handle(registry, &request.to_handle())
    })
}

pub(crate) fn delete_session_with_roots(
    registry: &ProviderRegistry,
    provider_id: &str,
    session_id: &str,
    source_path: &Path,
    roots: &[PathBuf],
) -> Result<bool, String> {
    let validated_source = canonicalize_existing_path(source_path, "session source")?;

    let mut saw_existing_root = false;
    for root in roots {
        if !root.exists() {
            continue;
        }

        saw_existing_root = true;
        let validated_root = canonicalize_existing_path(root, "session root")?;
        if validated_source.starts_with(&validated_root) {
            // Validate session_id via provider before trashing
            let provider = registry.get(provider_id)?;
            provider.validate_session_id(&validated_source, session_id)?;

            // Send to system trash (Recycle Bin on Windows, Trash on macOS/Linux)
            send_to_system_trash(&validated_source)?;
            return Ok(true);
        }
    }

    if !saw_existing_root {
        return Err(format!(
            "Session root not found for provider {provider_id}: {}",
            roots
                .first()
                .map(|root| root.display().to_string())
                .unwrap_or_else(|| "<none>".to_string())
        ));
    }

    Err(format!(
        "Session source path is outside provider roots: {}",
        source_path.display()
    ))
}

fn send_to_system_trash(source_path: &Path) -> Result<(), String> {
    // Collect all paths to trash: the JSONL file + its sidecar directory (if any)
    let mut paths: Vec<std::path::PathBuf> = Vec::with_capacity(2);
    paths.push(source_path.to_path_buf());

    if let Some(stem) = source_path.file_stem() {
        let sidecar = source_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(stem);
        if sidecar.exists() {
            paths.push(sidecar);
        }
    }

    trash::delete_all(&paths)
        .map_err(|e| format!("Failed to send session to system trash: {e}"))?;
    Ok(())
}

fn canonicalize_existing_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.exists() {
        return Err(format!("{label} not found: {}", path.display()));
    }

    path.canonicalize()
        .map_err(|e| format!("Failed to resolve {label} {}: {e}", path.display()))
}

pub(crate) fn collect_session_outcomes<F>(
    requests: &[DeleteSessionRequest],
    false_message: &str,
    mut operation: F,
) -> Vec<DeleteSessionOutcome>
where
    F: FnMut(&DeleteSessionRequest) -> Result<bool, String>,
{
    requests
        .iter()
        .map(|request| match operation(request) {
            Ok(true) => DeleteSessionOutcome {
                provider_id: request.provider_id.clone(),
                session_id: request.session_id.clone(),
                source_path: request.source_path.clone(),
                success: true,
                error: None,
            },
            Ok(false) => DeleteSessionOutcome {
                provider_id: request.provider_id.clone(),
                session_id: request.session_id.clone(),
                source_path: request.source_path.clone(),
                success: false,
                error: Some(false_message.to_string()),
            },
            Err(error) => DeleteSessionOutcome {
                provider_id: request.provider_id.clone(),
                session_id: request.session_id.clone(),
                source_path: request.source_path.clone(),
                success: false,
                error: Some(error),
            },
        })
        .collect()
}

/// Move a session between two root directories (e.g., active <-> archived).
/// Validates that `source` is under `from_root`, computes the relative path,
/// reconstructs the destination under `to_root`, validates the session ID,
/// then delegates to the provider's move logic.
fn move_session_between_roots(
    registry: &ProviderRegistry,
    provider_id: &str,
    session_id: &str,
    source_path: &str,
    from_root: &Path,
    to_root: &Path,
) -> Result<bool, String> {
    let source = Path::new(source_path);
    let validated_source = canonicalize_existing_path(source, "session source")?;
    let validated_from_root = canonicalize_existing_path(from_root, "source root")?;

    // Verify source is under from_root
    if !validated_source.starts_with(&validated_from_root) {
        return Err(format!(
            "Source is not in the expected directory: {}",
            source_path
        ));
    }

    // Compute relative path from from_root
    let relative = validated_source
        .strip_prefix(&validated_from_root)
        .map_err(|_| "Failed to compute relative path".to_string())?;

    let dest_dir = if let Some(parent) = relative.parent() {
        if parent.as_os_str().is_empty() {
            to_root.to_path_buf()
        } else {
            to_root.join(parent)
        }
    } else {
        to_root.to_path_buf()
    };

    // Validate session ID via the provider
    let provider = registry.get(provider_id)?;
    provider.validate_session_id(&validated_source, session_id)?;

    // Delegate the actual file move to the provider
    provider.move_session(&validated_source, &dest_dir)?;
    Ok(true)
}

#[allow(dead_code)]
#[deprecated(note = "use archive_session_for_handle instead")]
pub fn archive_session(
    registry: &ProviderRegistry,
    provider_id: &str,
    session_id: &str,
    source_path: &str,
) -> Result<bool, String> {
    let handle = SessionHandle {
        provider_id: provider_id.to_string(),
        session_id: session_id.to_string(),
        locator: super::types::SessionLocator::File {
            path: source_path.to_string(),
        },
    };
    archive_session_for_handle(registry, &handle)
}

pub fn archive_session_for_handle(
    registry: &ProviderRegistry,
    handle: &SessionHandle,
) -> Result<bool, String> {
    let source_path = handle
        .file_path()
        .map_err(|_| DB_OPERATION_UNSUPPORTED.to_string())?;
    let provider = registry.get(&handle.provider_id)?;
    let roots = provider.roots();
    // Active root = first root, Archive root = second root (per ClaudeProvider::roots())
    let active_root = roots
        .first()
        .ok_or_else(|| "No active root found".to_string())?;
    let archive_root = roots
        .get(1)
        .ok_or_else(|| "No archive root found".to_string())?;
    move_session_between_roots(
        registry,
        &handle.provider_id,
        &handle.session_id,
        source_path,
        active_root,
        archive_root,
    )
}

#[allow(dead_code)]
#[deprecated(note = "use restore_session_for_handle instead")]
pub fn restore_session(
    registry: &ProviderRegistry,
    provider_id: &str,
    session_id: &str,
    source_path: &str,
) -> Result<bool, String> {
    let handle = SessionHandle {
        provider_id: provider_id.to_string(),
        session_id: session_id.to_string(),
        locator: super::types::SessionLocator::File {
            path: source_path.to_string(),
        },
    };
    restore_session_for_handle(registry, &handle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TEST_ENV_LOCK;
    use tempfile::tempdir;

    // Use the global shared lock to prevent parallel tests from racing on
    // CLAUDE_CONFIG_DIR (same pattern as the tests in mod.rs).
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

    fn write_claude_session(path: &Path, session_id: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("create dir");
        std::fs::write(
            path,
            format!(
                "{{\"sessionId\":\"{session_id}\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}}\n\
                 {{\"message\":{{\"role\":\"user\",\"content\":\"hello\"}},\"timestamp\":\"2026-03-06T10:01:00Z\"}}\n",
            ),
        )
        .expect("write source");
    }

    fn request_for(path: &Path, session_id: &str) -> DeleteSessionRequest {
        DeleteSessionRequest {
            provider_id: "claude".to_string(),
            session_id: session_id.to_string(),
            source_path: path.to_string_lossy().to_string(),
            locator: Some(super::super::types::SessionLocator::File {
                path: path.to_string_lossy().to_string(),
            }),
        }
    }

    #[test]
    fn batch_archive_moves_all_sessions_to_archive_root() {
        let _guard = ENV_LOCK.lock().expect("lock");
        let cfg = tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("CLAUDE_CONFIG_DIR", cfg.path());
        let registry = build_provider_registry_for_tests();

        let projects = cfg.path().join("projects");
        let archived = cfg.path().join("projects_archived");
        let s1 = projects.join("folder-a").join("s1.jsonl");
        let s2 = projects.join("folder-b").join("s2.jsonl");
        write_claude_session(&s1, "id-1");
        write_claude_session(&s2, "id-2");

        let outcomes = archive_sessions(
            &registry,
            &[request_for(&s1, "id-1"), request_for(&s2, "id-2")],
        );

        assert_eq!(outcomes.len(), 2);
        for outcome in &outcomes {
            assert!(outcome.success, "expected success: {:?}", outcome.error);
            assert_eq!(outcome.error, None);
        }
        assert!(archived.join("folder-a").join("s1.jsonl").exists());
        assert!(archived.join("folder-b").join("s2.jsonl").exists());
        assert!(!s1.exists(), "source should be gone from active root");
        assert!(!s2.exists(), "source should be gone from active root");
    }

    #[test]
    fn batch_restore_moves_all_sessions_back_to_active_root() {
        let _guard = ENV_LOCK.lock().expect("lock");
        let cfg = tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("CLAUDE_CONFIG_DIR", cfg.path());
        let registry = build_provider_registry_for_tests();

        let projects = cfg.path().join("projects");
        let archived = cfg.path().join("projects_archived");
        let a1 = archived.join("folder-a").join("s1.jsonl");
        let a2 = archived.join("folder-b").join("s2.jsonl");
        write_claude_session(&a1, "id-1");
        write_claude_session(&a2, "id-2");

        let outcomes = restore_sessions(
            &registry,
            &[request_for(&a1, "id-1"), request_for(&a2, "id-2")],
        );

        assert_eq!(outcomes.len(), 2);
        for outcome in &outcomes {
            assert!(outcome.success, "expected success: {:?}", outcome.error);
        }
        assert!(projects.join("folder-a").join("s1.jsonl").exists());
        assert!(projects.join("folder-b").join("s2.jsonl").exists());
        assert!(!a1.exists(), "source should be gone from archive root");
        assert!(!a2.exists(), "source should be gone from archive root");
    }

    #[test]
    fn batch_archive_reports_missing_source_without_aborting_batch() {
        let _guard = ENV_LOCK.lock().expect("lock");
        let cfg = tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("CLAUDE_CONFIG_DIR", cfg.path());
        let registry = build_provider_registry_for_tests();

        let projects = cfg.path().join("projects");
        let archived = cfg.path().join("projects_archived");
        let valid = projects.join("folder-a").join("valid.jsonl");
        let stale = projects.join("folder-a").join("stale.jsonl");
        write_claude_session(&valid, "valid-id");
        // stale.jsonl is never written: simulates a sourcePath that went away
        // between listing and the batch operation.

        let outcomes = archive_sessions(
            &registry,
            &[request_for(&valid, "valid-id"), request_for(&stale, "stale-id")],
        );

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes[0].success, "valid item must still move");
        assert_eq!(outcomes[0].error, None);
        assert!(archived.join("folder-a").join("valid.jsonl").exists());
        assert!(!valid.exists());

        assert!(!outcomes[1].success, "missing source must be reported");
        assert!(outcomes[1]
            .error
            .as_deref()
            .expect("error string")
            .contains("session source not found"));
    }

    #[test]
    fn batch_archive_reports_unknown_provider_without_aborting_batch() {
        let _guard = ENV_LOCK.lock().expect("lock");
        let cfg = tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("CLAUDE_CONFIG_DIR", cfg.path());
        let registry = build_provider_registry_for_tests();

        let projects = cfg.path().join("projects");
        let archived = cfg.path().join("projects_archived");
        let valid = projects.join("folder-a").join("valid.jsonl");
        write_claude_session(&valid, "valid-id");

        let mut bogus = request_for(&valid, "other-id");
        bogus.provider_id = "no-such-provider".to_string();
        // Give the bogus item its own (nonexistent) path so assertions on the
        // valid item's file movement stay unambiguous.
        bogus.source_path = projects.join("folder-a").join("other.jsonl").to_string_lossy().to_string();

        let outcomes = archive_sessions(&registry, &[request_for(&valid, "valid-id"), bogus]);

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes[0].success, "valid item must still move");
        assert!(archived.join("folder-a").join("valid.jsonl").exists());
        assert!(!outcomes[1].success, "unknown provider must be reported");
        assert_eq!(
            outcomes[1].error.as_deref(),
            Some("Unknown provider: no-such-provider")
        );
    }

    #[test]
    fn batch_archive_rejects_session_id_mismatch_and_file_survives() {
        let _guard = ENV_LOCK.lock().expect("lock");
        let cfg = tempdir().expect("tempdir");
        let _env = EnvVarGuard::set_path("CLAUDE_CONFIG_DIR", cfg.path());
        let registry = build_provider_registry_for_tests();

        let projects = cfg.path().join("projects");
        let archived = cfg.path().join("projects_archived");
        let source = projects.join("folder-a").join("s1.jsonl");
        write_claude_session(&source, "real-id");

        let outcomes = archive_sessions(
            &registry,
            &[request_for(&source, "wrong-id"), request_for(&source, "real-id")],
        );

        assert_eq!(outcomes.len(), 2);
        // First item: mismatch rejected, file untouched at that point.
        assert!(!outcomes[0].success, "mismatched id must be rejected");
        let err = outcomes[0].error.as_deref().expect("error string");
        assert!(
            err.contains("session ID mismatch") || err.contains("ID mismatch"),
            "error should mention ID mismatch: {err}"
        );
        // Second item (correct id) still succeeds: the batch does not abort.
        assert!(outcomes[1].success, "correct id must still move: {:?}", outcomes[1].error);
        assert!(archived.join("folder-a").join("s1.jsonl").exists());
        assert!(!source.exists(), "file ends up moved exactly once");
    }

    fn build_provider_registry_for_tests() -> ProviderRegistry {
        // A fresh registry with the real Claude provider: enough for the batch
        // operations under test (they only need &ProviderRegistry).
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(crate::session_manager::providers::claude::ClaudeProvider));
        registry
    }
}

pub fn restore_session_for_handle(
    registry: &ProviderRegistry,
    handle: &SessionHandle,
) -> Result<bool, String> {
    let source_path = handle
        .file_path()
        .map_err(|_| DB_OPERATION_UNSUPPORTED.to_string())?;
    let provider = registry.get(&handle.provider_id)?;
    let roots = provider.roots();
    let archive_root = roots
        .get(1)
        .ok_or_else(|| "No archive root found".to_string())?;
    let active_root = roots
        .first()
        .ok_or_else(|| "No active root found".to_string())?;
    move_session_between_roots(
        registry,
        &handle.provider_id,
        &handle.session_id,
        source_path,
        archive_root,
        active_root,
    )
}
