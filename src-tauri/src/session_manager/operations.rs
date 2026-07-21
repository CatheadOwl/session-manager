use std::path::{Path, PathBuf};

use super::providers::ProviderRegistry;
use super::types::{DeleteSessionOutcome, DeleteSessionRequest};

pub fn delete_session(
    registry: &ProviderRegistry,
    provider_id: &str,
    session_id: &str,
    source_path: &str,
) -> Result<bool, String> {
    let provider = registry.get(provider_id)?;
    let roots = provider.roots();
    delete_session_with_roots(
        registry,
        provider_id,
        session_id,
        Path::new(source_path),
        &roots,
    )
}

pub fn delete_sessions(
    registry: &ProviderRegistry,
    requests: &[DeleteSessionRequest],
) -> Vec<DeleteSessionOutcome> {
    collect_session_outcomes(requests, "Session was not deleted", |request| {
        delete_session(
            registry,
            &request.provider_id,
            &request.session_id,
            &request.source_path,
        )
    })
}

pub fn archive_sessions(
    registry: &ProviderRegistry,
    requests: &[DeleteSessionRequest],
) -> Vec<DeleteSessionOutcome> {
    collect_session_outcomes(requests, "Session was not archived", |request| {
        archive_session(
            registry,
            &request.provider_id,
            &request.session_id,
            &request.source_path,
        )
    })
}

pub fn restore_sessions(
    registry: &ProviderRegistry,
    requests: &[DeleteSessionRequest],
) -> Vec<DeleteSessionOutcome> {
    collect_session_outcomes(requests, "Session was not restored", |request| {
        restore_session(
            registry,
            &request.provider_id,
            &request.session_id,
            &request.source_path,
        )
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

pub fn archive_session(
    registry: &ProviderRegistry,
    provider_id: &str,
    session_id: &str,
    source_path: &str,
) -> Result<bool, String> {
    let provider = registry.get(provider_id)?;
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
        provider_id,
        session_id,
        source_path,
        active_root,
        archive_root,
    )
}

pub fn restore_session(
    registry: &ProviderRegistry,
    provider_id: &str,
    session_id: &str,
    source_path: &str,
) -> Result<bool, String> {
    let provider = registry.get(provider_id)?;
    let roots = provider.roots();
    let archive_root = roots
        .get(1)
        .ok_or_else(|| "No archive root found".to_string())?;
    let active_root = roots
        .first()
        .ok_or_else(|| "No active root found".to_string())?;
    move_session_between_roots(
        registry,
        provider_id,
        session_id,
        source_path,
        archive_root,
        active_root,
    )
}
