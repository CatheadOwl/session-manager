use super::providers::ProviderRegistry;
use super::types::{SessionMeta, SessionScope};
use std::time::Instant;

pub fn scan_sessions_with_scope(
    registry: &ProviderRegistry,
    scope: &SessionScope,
) -> Vec<SessionMeta> {
    let start = Instant::now();
    log::debug!("list_scan start scope={}", scope_label(scope));
    let mut sessions = Vec::new();
    let mut provider_count = 0usize;
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

fn scope_label(scope: &SessionScope) -> &'static str {
    match scope {
        SessionScope::Active => "active",
        SessionScope::Archived => "archived",
    }
}
