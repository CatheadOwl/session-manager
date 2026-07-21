use super::providers::ProviderRegistry;
use super::types::{SessionMeta, SessionScope};

pub fn scan_sessions_with_scope(
    registry: &ProviderRegistry,
    scope: &SessionScope,
) -> Vec<SessionMeta> {
    let mut sessions = Vec::new();
    for provider in registry.all() {
        let roots = provider.roots();
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
        if root.exists() {
            sessions.extend(provider.scan_sessions(root));
        }
    }
    sessions.sort_by(|a, b| {
        let a_ts = a.last_active_at.or(a.created_at).unwrap_or(0);
        let b_ts = b.last_active_at.or(b.created_at).unwrap_or(0);
        b_ts.cmp(&a_ts)
    });
    sessions
}
