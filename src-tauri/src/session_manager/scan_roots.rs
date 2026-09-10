//! Shared home-relative scan-root derivation (ADR 0011).
//!
//! The ONE function both scan lines consume to answer "which standard
//! provider subdirectories live under a source root": each registry
//! provider's `roots()` (LOCAL absolute paths, e.g.
//! `C:\Users\u\.claude\projects`) is stripped of the local home prefix,
//! with separators normalized to `/`, yielding a home-relative path
//! (`.claude/projects`). Joining that relative path onto ANY home-shaped
//! root — a remote machine's `$HOME` (expanded by the remote shell) or a
//! local extra source root (an alternate home, ADR 0011) — reproduces the
//! provider's standard directory there.
//!
//! Consumers:
//! - remote line (`remote/scan.rs`): `$HOME/<rel>` over SSH;
//! - local overlay (`scan.rs`): `entry_root.join(<rel>)` per provider.
//!
//! Scope semantics (copied from the local scan core, shared verbatim):
//! `roots()[0]` = active, `roots()[1]` = archived. A provider with no
//! root for the scope is skipped. Roots NOT under the local home prefix
//! (e.g. opencode's storage outside `~`) have no derivable counterpart
//! and are skipped with a debug log.
//!
//! This module is Tauri-free and makes no IO — it is pure path algebra.

use std::path::Path;

use crate::session_manager::providers::ProviderRegistry;
use crate::session_manager::types::SessionScope;

/// One derived scan root: `provider_id` owns the directory
/// `<source_root>/<rel>`, where `rel` is the provider's LOCAL root with
/// the home prefix stripped and separators normalized to `/` (empty `rel`
/// means the source root itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedRoot {
    pub provider_id: String,
    pub rel: String,
}

/// Derive the scan roots for `scope` from every registry provider's
/// `roots()`, against an explicit home prefix (the production path passes
/// the REAL local home; tests pass a temp "home" so fixtures are
/// OS-independent).
///
/// - `roots()[0]` for Active, `roots()[1]` for Archived (scope semantics
///   copied from the local scan core);
/// - a provider with no root for the scope (e.g. no archived root) is
///   skipped — local parity;
/// - roots not under `home` are skipped with a debug log (no derivable
///   counterpart under a home-shaped source root).
pub fn derive_scan_roots_with_home(
    registry: &ProviderRegistry,
    scope: &SessionScope,
    home: &Path,
) -> Vec<DerivedRoot> {
    let mut out = Vec::new();
    for provider in registry.all() {
        let roots = provider.roots();
        let root = match scope {
            SessionScope::Active => roots.first(),
            SessionScope::Archived => roots.get(1),
        };
        let Some(root) = root else {
            continue; // provider has no root for this scope (local parity)
        };
        match home_relative_posix(root, home) {
            Some(rel) => out.push(DerivedRoot {
                provider_id: provider.id().to_string(),
                rel,
            }),
            None => log::debug!(
                "scan roots derivation: provider `{}` root {} is not under the home prefix {} — no counterpart under a home-shaped source root, skipped",
                provider.id(),
                root.display(),
                home.display()
            ),
        }
    }
    out
}

/// Strip the `home` prefix from a local root and normalize to a posix
/// home-relative path (`C:\Users\u\.claude\projects` →
/// `.claude/projects`; `~` itself → `""`). `None` when the root is not
/// under `home` (component-wise comparison — a sibling directory like
/// `C:\Users\other` never matches a `C:\Users\u` home prefix).
pub fn home_relative_posix(root: &Path, home: &Path) -> Option<String> {
    let rel = root.strip_prefix(home).ok()?;
    Some(
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_manager::providers::SessionProvider;
    use crate::session_manager::types::{SessionMeta, SessionMessage};
    use std::path::PathBuf;
    use tempfile::tempdir;

    /// Minimal fixture provider (same shape as the scan/remote test
    /// seams): `roots()` is all that matters for derivation.
    struct FixtureProvider {
        id: &'static str,
        roots: Vec<PathBuf>,
    }

    impl SessionProvider for FixtureProvider {
        fn id(&self) -> &str {
            self.id
        }
        fn roots(&self) -> Vec<PathBuf> {
            self.roots.clone()
        }
        fn scan_sessions(&self, _root: &Path) -> Vec<SessionMeta> {
            Vec::new()
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

    fn registry_with(providers: Vec<FixtureProvider>) -> ProviderRegistry {
        let mut registry = ProviderRegistry::new();
        for p in providers {
            registry.register(Box::new(p));
        }
        registry
    }

    #[test]
    fn derive_strips_home_and_normalizes_separators() {
        let home = tempdir().expect("tempdir");
        let registry = registry_with(vec![FixtureProvider {
            id: "alpha",
            roots: vec![
                home.path().join(".alpha").join("projects"),
                home.path().join(".alpha").join("archived"),
            ],
        }]);
        let active = derive_scan_roots_with_home(&registry, &SessionScope::Active, home.path());
        assert_eq!(
            active,
            vec![DerivedRoot {
                provider_id: "alpha".to_string(),
                rel: ".alpha/projects".to_string(),
            }],
            "active = roots()[0], home-stripped, '/'-separated"
        );
        let archived = derive_scan_roots_with_home(&registry, &SessionScope::Archived, home.path());
        assert_eq!(
            archived,
            vec![DerivedRoot {
                provider_id: "alpha".to_string(),
                rel: ".alpha/archived".to_string(),
            }],
            "archived = roots()[1]"
        );
    }

    #[test]
    fn home_root_itself_is_empty_rel() {
        let home = tempdir().expect("tempdir");
        assert_eq!(
            home_relative_posix(home.path(), home.path()),
            Some("".to_string()),
            "root == home → empty rel → the source root itself"
        );
    }

    #[test]
    fn roots_outside_home_are_skipped() {
        let home = tempdir().expect("tempdir");
        let outside = tempdir().expect("tempdir outside");
        let registry = registry_with(vec![FixtureProvider {
            id: "opencode-like",
            roots: vec![outside.path().join("data")],
        }]);
        assert!(derive_scan_roots_with_home(&registry, &SessionScope::Active, home.path()).is_empty());
    }
}
