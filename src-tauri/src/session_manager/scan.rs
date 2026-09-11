use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::providers::ProviderRegistry;
use super::scan_roots;
use super::settings::SourceEntry;
use super::types::{SessionMeta, SessionScope};

/// `extra_sources` is the settings-core sources overlay: enabled local
/// extra roots from
/// `~/.session-manager/settings.json`, each an ALTERNATE HOME — the
/// scan mirrors it exactly like a remote machine's home:
/// every provider's standard root, derived home-relative by the shared
/// [`scan_roots`] module, is discovered under it. There is no
/// per-entry provider anymore — directory ownership decides.
/// The slice is an explicit parameter — NOT a `SettingsManager`
/// reference tucked into the registry — so this module stays
/// Tauri-free and unit-testable; callers read
/// `SettingsManager::enabled_sources()` and pass the slice through.
///
/// Overlay semantics:
/// - BOTH scopes: Active scans each provider's active root under the
///   extra home, Archived its archived root (a provider without an
///   archive root skips Archived — same rule as built-in);
/// - a provider whose standard subdirectory is absent under the extra
///   root contributes 0 sessions with NO warning (a backup home holding
///   only one provider's data is a normal shape — debug log only);
/// - missing extra root → warn + skip (the entry survives in settings,
///   D4 unchanged — correct semantics for removable drives);
/// - deduped per provider against the provider's `scan_roots()` and
///   previously processed overlay dirs, so a joined dir equal to a
///   built-in root (e.g. an extra root pointing at the real home)
///   never double-scans;
/// - ssh entries are skipped here (the remote scan line
///   owns their consumption);
/// - disabled entries are skipped defensively (the settings manager
///   already filters, but this function does not require it).
pub fn scan_sessions_with_scope(
    registry: &ProviderRegistry,
    scope: &SessionScope,
    extra_sources: &[SourceEntry],
) -> Vec<SessionMeta> {
    scan_sessions_with_scope_with_home(
        registry,
        scope,
        extra_sources,
        &crate::config::get_home_dir(),
    )
}

/// Test seam / core of [`scan_sessions_with_scope`] with an explicit
/// home prefix for the root derivation (production passes the REAL local
/// home; tests pass a temp "home" so fixtures are OS-independent — same
/// pattern as the remote scan core).
pub fn scan_sessions_with_scope_with_home(
    registry: &ProviderRegistry,
    scope: &SessionScope,
    extra_sources: &[SourceEntry],
    home: &Path,
) -> Vec<SessionMeta> {
    let start = Instant::now();
    log::debug!("list_scan start scope={}", scope_label(scope));
    let mut sessions = Vec::new();
    let mut provider_count = 0usize;
    // Normalized roots already covered this pass, per provider id. Seeded
    // from each provider's scan roots (Active: ALL of them, so the
    // overlay can dedupe against every built-in root; Archived: the
    // archive root actually scanned) — the overlay dedupes its joined
    // directories against the same sets.
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
        {
            // Seed the dedupe set with every root this scope could scan
            // for the provider (Active: all; Archived: the archive root).
            let covered = scanned_roots
                .entry(provider.id().to_string())
                .or_default();
            match scope {
                SessionScope::Active => {
                    for r in &roots {
                        covered.insert(normalize_root(r));
                    }
                }
                SessionScope::Archived => {
                    covered.insert(normalize_root(root));
                }
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
    // Home-relative derivation is shared with the remote line
    // and computed once — registry and scope are fixed for the pass.
    let derived_roots = scan_roots::derive_scan_roots_with_home(registry, scope, home);
    for entry in extra_sources.iter().filter(|e| e.is_enabled()) {
        // Only local entries flow through the synchronous
        // overlay; ssh entries belong to the remote scan line and are
        // skipped here.
        let SourceEntry::Local(entry) = entry else {
            log::debug!(
                "list_scan sources overlay: skipping non-local entry (owned by the remote scan line)"
            );
            continue;
        };
        let path = PathBuf::from(&entry.path);
        if !path.exists() {
            // Warn-only (D4): the entry stays in settings so the user can
            // fix the path by hand; a bad root must not fail the scan.
            log::warn!(
                "list_scan sources overlay: root does not exist, skipping path={}",
                path.display()
            );
            continue;
        }
        for derived in &derived_roots {
            let provider = match registry.get(&derived.provider_id) {
                Ok(p) => p,
                Err(err) => {
                    // Unreachable in practice (ids come from the registry
                    // itself); kept defensive for registry mutation.
                    log::warn!(
                        "list_scan sources overlay: skipping derived root ({})",
                        err
                    );
                    continue;
                }
            };
            // Empty rel (provider root == home itself) means the extra
            // root IS the provider directory.
            let joined = if derived.rel.is_empty() {
                path.clone()
            } else {
                path.join(&derived.rel)
            };
            let covered = scanned_roots
                .entry(provider.id().to_string())
                .or_default();
            if !covered.insert(normalize_root(&joined)) {
                log::debug!(
                    "list_scan sources overlay: duplicate root skipped provider={} path={}",
                    provider.id(),
                    joined.display()
                );
                continue;
            }
            if !joined.exists() {
                // Normal shape: the extra home simply has no
                // data for this provider — 0 sessions, no warn.
                log::debug!(
                    "list_scan sources overlay: no {} root under the extra home, provider={}",
                    scope_label(scope),
                    provider.id()
                );
                continue;
            }
            log::debug!(
                "list_scan provider={} scope={} root={} (settings source)",
                provider.id(),
                scope_label(scope),
                joined.display()
            );
            sessions.extend(provider.scan_sessions(&joined).into_iter().inspect(|meta| {
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

    /// Minimal fixture provider: `roots()` (home-mirror derivation input)
    /// is decoupled from `root` (where the canned session's meta claims
    /// to live); one canned session per scan with a configurable
    /// timestamp. The trait's load/parse/move methods are irrelevant to
    /// the scan loop and stubbed out.
    struct FixtureProvider {
        id: &'static str,
        /// Roots reported to the derivation (must sit under the injected
        /// fake home for the home-mirror overlay to find them).
        roots: Vec<PathBuf>,
        /// Directory the canned session's meta points under.
        root: PathBuf,
        session_id: &'static str,
        last_active_at: i64,
    }

    impl SessionProvider for FixtureProvider {
        fn id(&self) -> &str {
            self.id
        }

        fn roots(&self) -> Vec<PathBuf> {
            self.roots.clone()
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

    fn source(path: &Path) -> SourceEntry {
        SourceEntry::Local(super::super::settings::LocalSource {
            path: path.to_string_lossy().into_owned(),
            enabled: true,
            id: None,
            extra: std::collections::BTreeMap::new(),
        })
    }

    fn ssh_source(id: &str, host: &str, enabled: bool) -> SourceEntry {
        SourceEntry::Ssh(super::super::settings::SshSource {
            id: id.to_string(),
            label: None,
            host: host.to_string(),
            port: 22,
            user: "admin".to_string(),
            auth: super::super::settings::SourceAuth::Agent,
            enabled,
            extra: std::collections::BTreeMap::new(),
        })
    }

    fn registry_with(provider: FixtureProvider) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        registry.register(Box::new(provider));
        registry
    }

    #[test]
    fn overlay_skips_missing_path() {
        let home = tempdir().expect("tempdir home");
        // The provider's built-in root sits under the (fake) home.
        let builtin = home.path().join(".alpha").join("projects");
        std::fs::create_dir_all(&builtin).expect("mkdir");
        let registry = registry_with(FixtureProvider {
            id: "alpha",
            roots: vec![
                builtin.clone(),
                home.path().join(".alpha").join("archived"),
            ],
            root: builtin.clone(),
            session_id: "builtin",
            last_active_at: 100,
        });

        let missing = home.path().join("does-not-exist");

        let sessions = scan_sessions_with_scope_with_home(
            &registry,
            &SessionScope::Active,
            &[source(&missing)],
            home.path(),
        );

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "builtin");
    }

    #[test]
    fn overlay_dedupes_builtin_root_when_extra_root_is_home_itself() {
        // An extra root equal to the real home: every joined dir IS a
        // built-in root — the dedupe set must swallow it (no double scan),
        // including a duplicated extra entry.
        let home = tempdir().expect("tempdir home");
        let builtin = home.path().join(".alpha").join("projects");
        std::fs::create_dir_all(&builtin).expect("mkdir");
        let registry = registry_with(FixtureProvider {
            id: "alpha",
            roots: vec![builtin.clone(), home.path().join(".alpha").join("archived")],
            root: builtin.clone(),
            session_id: "builtin",
            last_active_at: 100,
        });

        let sessions = scan_sessions_with_scope_with_home(
            &registry,
            &SessionScope::Active,
            &[source(home.path()), source(home.path())],
            home.path(),
        );

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "builtin");
    }

    #[test]
    fn overlay_appends_extra_home_and_sorts_by_last_active_at() {
        let home = tempdir().expect("tempdir home");
        // alpha's built-in root exists under the fake home (its canned
        // session is the older one).
        let alpha_builtin = home.path().join(".alpha").join("projects");
        std::fs::create_dir_all(&alpha_builtin).expect("mkdir");
        let extra = tempdir().expect("tempdir extra");
        // Extra home layout: `.beta/projects` present (discovered), no
        // `.alpha` at all (0 sessions for alpha, NO warn).
        let extra_beta = extra.path().join(".beta").join("projects");
        std::fs::create_dir_all(&extra_beta).expect("mkdir");

        let mut registry = registry_with(FixtureProvider {
            id: "beta",
            roots: vec![
                home.path().join(".beta").join("projects"),
                home.path().join(".beta").join("archived"),
            ],
            root: PathBuf::from("Z:/nonexistent-beta-root"),
            session_id: "newer",
            last_active_at: 200,
        });
        registry.register(Box::new(FixtureProvider {
            id: "alpha",
            roots: vec![
                alpha_builtin.clone(),
                home.path().join(".alpha").join("archived"),
            ],
            root: alpha_builtin.clone(),
            session_id: "builtin",
            last_active_at: 100,
        }));

        let sessions = scan_sessions_with_scope_with_home(
            &registry,
            &SessionScope::Active,
            &[source(extra.path())],
            home.path(),
        );

        // alpha contributes ONLY its built-in root (no `.alpha` under the
        // extra home); beta contributes ONLY via the overlay — its
        // session is NEWER than alpha's, proving append + global sort in
        // one pass.
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "newer");
        assert_eq!(sessions[0].provider_id, "beta");
        assert_eq!(sessions[1].session_id, "builtin");
        assert_eq!(sessions[1].provider_id, "alpha");
    }

    #[test]
    fn overlay_discovers_multiple_providers_under_one_extra_home() {
        let home = tempdir().expect("tempdir home");
        let extra = tempdir().expect("tempdir extra");
        std::fs::create_dir_all(extra.path().join(".alpha").join("projects")).expect("mkdir");
        std::fs::create_dir_all(extra.path().join(".beta").join("projects")).expect("mkdir");

        let mut registry = registry_with(FixtureProvider {
            id: "alpha",
            roots: vec![home.path().join(".alpha").join("projects")],
            root: home.path().join(".alpha").join("projects"),
            session_id: "a-extra",
            last_active_at: 100,
        });
        registry.register(Box::new(FixtureProvider {
            id: "beta",
            roots: vec![home.path().join(".beta").join("projects")],
            root: home.path().join(".beta").join("projects"),
            session_id: "b-extra",
            last_active_at: 200,
        }));

        let sessions = scan_sessions_with_scope_with_home(
            &registry,
            &SessionScope::Active,
            &[source(extra.path())],
            home.path(),
        );

        // Both providers discovered under the same extra home (directory
        // ownership decides — no per-entry provider).
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "b-extra");
        assert_eq!(sessions[1].session_id, "a-extra");
    }

    #[test]
    fn overlay_covers_archived_scope_via_the_archive_root() {
        // Extra homes are scanned in BOTH scopes; the
        // Archived pass derives `roots()[1]` per provider.
        let home = tempdir().expect("tempdir home");
        let extra = tempdir().expect("tempdir extra");
        // Archived layout under the extra home only (`.alpha/archived`).
        let extra_archived = extra.path().join(".alpha").join("archived");
        std::fs::create_dir_all(&extra_archived).expect("mkdir");

        let registry = registry_with(FixtureProvider {
            id: "alpha",
            roots: vec![
                home.path().join(".alpha").join("projects"),
                home.path().join(".alpha").join("archived"),
            ],
            root: extra_archived.clone(),
            session_id: "archived-extra",
            last_active_at: 300,
        });

        let archived = scan_sessions_with_scope_with_home(
            &registry,
            &SessionScope::Archived,
            &[source(extra.path())],
            home.path(),
        );
        assert_eq!(archived.len(), 1);
        assert_eq!(archived[0].session_id, "archived-extra");

        // The Active pass derives `.alpha/projects` — absent under the
        // extra home → 0 sessions there.
        let active = scan_sessions_with_scope_with_home(
            &registry,
            &SessionScope::Active,
            &[source(extra.path())],
            home.path(),
        );
        assert!(active.is_empty());
    }

    #[test]
    fn overlay_ignores_disabled_entries_and_ssh_entries() {
        let home = tempdir().expect("tempdir home");
        let extra = tempdir().expect("tempdir extra");
        let registry = registry_with(FixtureProvider {
            id: "alpha",
            roots: vec![
                home.path().join(".alpha").join("projects"),
                home.path().join(".alpha").join("archived"),
            ],
            root: home.path().join(".alpha").join("projects"),
            session_id: "builtin",
            last_active_at: 100,
        });

        let mut entries = vec![source(extra.path())];
        if let SourceEntry::Local(local) = &mut entries[0] {
            local.enabled = false;
        }
        // Enabled ssh entries reach the overlay but must be
        // skipped (remote line owns their consumption).
        entries.push(ssh_source("ali", "192.0.2.10", true));
        let active = scan_sessions_with_scope_with_home(
            &registry,
            &SessionScope::Active,
            &entries,
            home.path(),
        );
        assert!(active.is_empty());
    }
}
