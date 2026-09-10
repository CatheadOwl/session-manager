use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::providers::ProviderRegistry;
use super::settings::SourceEntry;
use super::types::{SessionMeta, SessionScope};

/// `extra_sources` is the settings-core sources overlay (ADR 0006, D2):
/// enabled extra scan roots from `~/.session-manager/settings.json`, each
/// naming the provider that owns the parser (D5). It is an explicit
/// parameter — NOT a `SettingsManager` reference tucked into the registry —
/// so this module stays Tauri-free and unit-testable; callers read
/// `SettingsManager::enabled_sources()` and pass the slice through.
///
/// Overlay semantics:
/// - active scope ONLY: a settings source has no archive root, so archived
///   scans ignore the overlay entirely;
/// - unknown provider id → warn + skip (warn-only, D4);
/// - missing path → warn + skip (the entry survives in settings);
/// - deduped per provider against ALL of that provider's `scan_roots()` and
///   against previously processed entries, so a source equal to a built-in
///   root never double-scans;
/// - disabled entries are skipped defensively (the settings manager already
///   filters, but this function does not require it).
pub fn scan_sessions_with_scope(
    registry: &ProviderRegistry,
    scope: &SessionScope,
    extra_sources: &[SourceEntry],
) -> Vec<SessionMeta> {
    let start = Instant::now();
    log::debug!("list_scan start scope={}", scope_label(scope));
    let mut sessions = Vec::new();
    let mut provider_count = 0usize;
    // Normalized roots already covered this pass, per provider id. Seeded
    // from each provider's full scan_roots() so the overlay can dedupe
    // against every built-in root, not just the one picked for this scope.
    let mut scanned_roots: HashMap<String, HashSet<PathBuf>> = HashMap::new();
    for provider in registry.all() {
        provider_count += 1;
        let roots = provider.scan_roots();
        if roots.is_empty() {
            continue;
        }
        let root = match scope {
            SessionScope::Active => &roots[0],
            SessionScope::Archived => {
                if roots.len() < 2 {
                    continue; // provider has no archive directory
                }
                &roots[1]
            }
        };
        if matches!(scope, SessionScope::Active) {
            let covered = scanned_roots
                .entry(provider.id().to_string())
                .or_default();
            for r in &roots {
                covered.insert(normalize_root(r));
            }
        }
        log::debug!(
            "list_scan provider={} scope={} root={}",
            provider.id(),
            scope_label(scope),
            root.display()
        );
        if root.exists() {
            sessions.extend(provider.scan_sessions(root).into_iter().inspect(|meta| {
                meta.debug_assert_file_locator_matches_source_path();
            }));
        }
    }
    if matches!(scope, SessionScope::Active) {
        for entry in extra_sources.iter().filter(|e| e.enabled) {
            let provider = match registry.get(&entry.provider) {
                Ok(p) => p,
                Err(err) => {
                    log::warn!(
                        "list_scan sources overlay: skipping entry path={} ({})",
                        entry.path,
                        err
                    );
                    continue;
                }
            };
            let path = PathBuf::from(&entry.path);
            let covered = scanned_roots
                .entry(provider.id().to_string())
                .or_default();
            if !covered.insert(normalize_root(&path)) {
                log::debug!(
                    "list_scan sources overlay: duplicate root skipped provider={} path={}",
                    provider.id(),
                    path.display()
                );
                continue;
            }
            if !path.exists() {
                // Warn-only (D4): the entry stays in settings so the user can
                // fix the path by hand; a bad root must not fail the scan.
                log::warn!(
                    "list_scan sources overlay: root does not exist, skipping provider={} path={}",
                    provider.id(),
                    path.display()
                );
                continue;
            }
            log::debug!(
                "list_scan provider={} scope={} root={} (settings source)",
                provider.id(),
                scope_label(scope),
                path.display()
            );
            sessions.extend(provider.scan_sessions(&path).into_iter().inspect(|meta| {
                meta.debug_assert_file_locator_matches_source_path();
            }));
        }
    }
    let total = sessions.len();
    sessions.sort_by(|a, b| {
        let a_ts = a.last_active_at.or(a.created_at).unwrap_or(0);
        let b_ts = b.last_active_at.or(b.created_at).unwrap_or(0);
        b_ts.cmp(&a_ts)
    });
    log::debug!(
        "list_scan finish scope={} provider_count={} session_count={} elapsed_ms={}",
        scope_label(scope),
        provider_count,
        total,
        start.elapsed().as_millis()
    );
    sessions
}

/// Cheap dedupe key: canonical form when the path is on disk, the path as
/// given otherwise (missing paths never reach the comparison against real
/// roots anyway — they are warned and skipped first).
fn normalize_root(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn scope_label(scope: &SessionScope) -> &'static str {
    match scope {
        SessionScope::Active => "active",
        SessionScope::Archived => "archived",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::providers::SessionProvider;
    use crate::session_manager::{SessionLocator, SessionMessage};
    use tempfile::tempdir;

    /// Minimal fixture provider: one root directory, one canned session per
    /// scan with a configurable timestamp. The trait's load/parse/move
    /// methods are irrelevant to the scan loop and stubbed out.
    struct FixtureProvider {
        id: &'static str,
        root: PathBuf,
        session_id: &'static str,
        last_active_at: i64,
    }

    impl SessionProvider for FixtureProvider {
        fn id(&self) -> &str {
            self.id
        }

        fn roots(&self) -> Vec<PathBuf> {
            vec![self.root.clone()]
        }

        fn scan_sessions(&self, root: &Path) -> Vec<SessionMeta> {
            vec![meta(self.id, self.session_id, root, self.last_active_at)]
        }

        fn load_messages(&self, _path: &Path) -> Result<Vec<SessionMessage>, String> {
            Ok(Vec::new())
        }

        fn load_raw_content_fallback(&self, _path: &Path) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn parse_session(&self, _path: &Path) -> Option<SessionMeta> {
            None
        }

        fn move_session(&self, _source: &Path, _dest: &Path) -> Result<(), String> {
            Ok(())
        }
    }

    fn meta(provider: &str, id: &str, root: &Path, last_active_at: i64) -> SessionMeta {
        let file = root.join(format!("{id}.jsonl"));
        let path = file.to_string_lossy().into_owned();
        SessionMeta {
            provider_id: provider.to_string(),
            session_id: id.to_string(),
            title: None,
            summary: None,
            project_dir: None,
            created_at: Some(last_active_at),
            last_active_at: Some(last_active_at),
            source_path: Some(path.clone()),
            locator: Some(SessionLocator::File { path }),
            resume_command: None,
            forked_from_id: None,
        }
    }

    fn source(path: &Path, provider: &str) -> SourceEntry {
        SourceEntry {
            path: path.to_string_lossy().into_owned(),
            provider: provider.to_string(),
            enabled: true,
        }
    }

    fn registry_with(provider: FixtureProvider) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(provider));
        registry
    }

    #[test]
    fn overlay_skips_unknown_provider() {
        let root = tempdir().expect("tempdir");
        let registry = registry_with(FixtureProvider {
            id: "alpha",
            root: root.path().to_path_buf(),
            session_id: "builtin",
            last_active_at: 100,
        });

        let sessions = scan_sessions_with_scope(
            &registry,
            &SessionScope::Active,
            &[source(root.path(), "no-such-provider")],
        );

        // Only the built-in root's session; the unknown-provider entry is
        // warned and skipped, not an error.
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "builtin");
    }

    #[test]
    fn overlay_skips_missing_path() {
        let root = tempdir().expect("tempdir");
        let registry = registry_with(FixtureProvider {
            id: "alpha",
            root: root.path().to_path_buf(),
            session_id: "builtin",
            last_active_at: 100,
        });
        let missing = root.path().join("does-not-exist");

        let sessions = scan_sessions_with_scope(
            &registry,
            &SessionScope::Active,
            &[source(&missing, "alpha")],
        );

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "builtin");
    }

    #[test]
    fn overlay_dedupes_builtin_root() {
        let root = tempdir().expect("tempdir");
        let registry = registry_with(FixtureProvider {
            id: "alpha",
            root: root.path().to_path_buf(),
            session_id: "builtin",
            last_active_at: 100,
        });

        // Same root as the provider's built-in scan root: must not scan twice.
        let sessions = scan_sessions_with_scope(
            &registry,
            &SessionScope::Active,
            &[source(root.path(), "alpha"), source(root.path(), "alpha")],
        );

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "builtin");
    }

    #[test]
    fn overlay_appends_extra_root_and_sorts_by_last_active_at() {
        let root = tempdir().expect("tempdir");
        let extra = tempdir().expect("tempdir extra");
        let registry = registry_with(FixtureProvider {
            id: "alpha",
            root: root.path().to_path_buf(),
            session_id: "builtin",
            last_active_at: 100,
        });

        // beta's built-in root intentionally does not exist, so beta
        // contributes ONLY via the overlay entry — whose session is NEWER
        // than alpha's, proving append + global sort in one pass.
        let mut registry = registry_with(FixtureProvider {
            id: "beta",
            root: PathBuf::from("Z:/nonexistent-beta-root"),
            session_id: "newer",
            last_active_at: 200,
        });
        registry.register(Box::new(FixtureProvider {
            id: "alpha",
            root: root.path().to_path_buf(),
            session_id: "builtin",
            last_active_at: 100,
        }));

        let sessions = scan_sessions_with_scope(
            &registry,
            &SessionScope::Active,
            &[source(extra.path(), "beta")],
        );

        assert_eq!(sessions.len(), 2);
        // Sorted by last_active_at descending across built-in + overlay.
        assert_eq!(sessions[0].session_id, "newer");
        assert_eq!(sessions[1].session_id, "builtin");
    }

    #[test]
    fn overlay_ignores_disabled_entries_and_archived_scope() {
        let root = tempdir().expect("tempdir");
        let extra = tempdir().expect("tempdir extra");
        let registry = registry_with(FixtureProvider {
            id: "alpha",
            root: root.path().to_path_buf(),
            session_id: "builtin",
            last_active_at: 100,
        });

        let mut disabled = source(extra.path(), "alpha");
        disabled.enabled = false;
        let active = scan_sessions_with_scope(&registry, &SessionScope::Active, &[disabled]);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].session_id, "builtin");

        // Sources have no archive root: the archived scan ignores the overlay.
        let archived = scan_sessions_with_scope(
            &registry,
            &SessionScope::Archived,
            &[source(extra.path(), "alpha")],
        );
        assert!(archived.is_empty());
    }
}
