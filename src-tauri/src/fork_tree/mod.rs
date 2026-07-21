pub mod cache;
pub mod filter;
pub mod hash_chain;
pub mod tree_builder;
pub mod types;

#[allow(unused_imports)]
pub use types::{ForkTreeResult, TreeNodeData};

use std::path::Path;
use std::time::Instant;

use self::types::CachedFileData;

use crate::config;
use crate::session_manager;
use crate::session_manager::providers::ProviderRegistry;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Compute (or fetch from cache) the fork tree for a given scope.
/// `project_dir_filter` — if set, only include sessions whose `project_dir` matches
/// (case-insensitive comparison on Windows). Pass `None` to include all sessions.
pub fn compute_fork_tree(
    registry: &ProviderRegistry,
    scope: &session_manager::SessionScope,
    project_dir_filter: Option<&str>,
) -> Result<ForkTreeResult, String> {
    let start = Instant::now();

    let cache_path = config::get_fork_tree_cache_path()?;

    // Get the already-filtered session list from the session manager.
    // This skips subagent sessions and other files the provider rejects.
    let sessions = session_manager::scan_sessions_with_scope(registry, scope);

    // Load existing cache
    let mut cache = cache::load_cache(&cache_path);

    // Build O(1) lookup: source_path → index in cache.files
    let cache_index: std::collections::HashMap<String, usize> = cache
        .files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.source_path.clone(), i))
        .collect();

    // Build current_files from session results, reusing cache where possible
    let mut current_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut current_files: Vec<CachedFileData> = Vec::new();

    for session in &sessions {
        let source_path = match &session.source_path {
            Some(p) => p,
            None => continue,
        };
        current_paths.insert(source_path.clone());

        // Reuse cached hash-chain data if available for this source path.
        // The hash chain is expensive to recompute (full-file read + SHA256),
        // but session meta fields are cheap — override those so changes to
        // forked_from_id, title, summary, or last_active_at are reflected
        // immediately without a manual cache clear.
        if let Some(&cached_idx) = cache_index.get(source_path) {
            let mut data = cache.files[cached_idx].clone();
            data.forked_from_id = session.forked_from_id.clone();
            data.title = session.title.clone().unwrap_or_else(|| {
                session.session_id.chars().take(8).collect()
            });
            data.summary = session.summary.clone();
            data.last_active_at = session.last_active_at;
            data.project_dir = session.project_dir.clone();
            current_files.push(data);
            continue;
        }

        // Compute hash chain for this session
        let provider = match registry.get(&session.provider_id) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Warning: provider not found for session {}: {e}", source_path);
                continue;
            }
        };

        let events = match provider.user_events(Path::new(source_path)) {
            Ok(events) => events,
            Err(e) => {
                eprintln!("Warning: failed to compute fork data for {}: {e}", source_path);
                continue;
            }
        };

        let (hash_chain, user_texts, kept_indices) = hash_chain::hash_events(&events);

        let file_data = CachedFileData {
            session_key: format!(
                "{}:{}:{}",
                session.provider_id, session.session_id, source_path
            ),
            source_path: source_path.clone(),
            title: session.title.clone().unwrap_or_else(|| {
                session.session_id.chars().take(8).collect()
            }),
            summary: session.summary.clone(),
            last_active_at: session.last_active_at,
            project_dir: session.project_dir.clone(),
            hash_chain,
            user_texts,
            kept_indices,
            forked_from_id: session.forked_from_id.clone(),
            uuid_chain: vec![],
        };

        current_files.push(file_data.clone());
        cache.files.push(file_data);
    }

    // Prune stale cache entries (sessions no longer in scan results)
    cache.files.retain(|f| current_paths.contains(&f.source_path));

    // Filter and rebuild tree
    let (roots, total_sessions) = if let Some(dir) = project_dir_filter {
        let filtered = filter::filter_files_by_project_dir(&current_files, dir);
        let roots = tree_builder::build_tree(&filtered);
        (roots, filtered.len() as u32)
    } else {
        let roots = tree_builder::build_tree(&current_files);
        (roots, current_files.len() as u32)
    };

    // Save cache
    cache::save_cache(&cache_path, &cache)?;

    Ok(ForkTreeResult {
        total_sessions,
        roots,
        computed_from_cache: false,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Return cached fork tree without recomputing.
pub fn get_fork_tree() -> Result<ForkTreeResult, String> {
    let start = Instant::now();
    let cache_path = config::get_fork_tree_cache_path()?;
    let cache = cache::load_cache(&cache_path);

    if cache.files.is_empty() {
        return Ok(ForkTreeResult {
            roots: Vec::new(),
            total_sessions: 0,
            computed_from_cache: true,
            duration_ms: 0,
        });
    }

    let roots = tree_builder::build_tree(&cache.files);
    Ok(ForkTreeResult {
        roots,
        total_sessions: cache.files.len() as u32,
        computed_from_cache: true,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::hash_chain::{
        compute_file_data, hash_events, sha256_first_8, USER_TEXT_PREVIEW_CHARS,
    };
    use super::tree_builder::build_tree;
    use super::types::{CachedFileData, TreeNodeData};
    use super::{compute_fork_tree, get_fork_tree};

    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::tempdir;

    use crate::config::TEST_ENV_LOCK;
    use crate::session_manager;
    use crate::session_manager::build_provider_registry;

    // Use the global shared lock to prevent parallel tests from racing on env vars.
    static ENV_LOCK: &std::sync::Mutex<()> = &TEST_ENV_LOCK;

    #[test]
    fn sha256_first_8_produces_8_char_hex() {
        let hash = sha256_first_8("hello");
        assert_eq!(hash.len(), 8);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sha256_matches_analysis_ps1_behavior() {
        // Verify deterministic output
        let a = sha256_first_8("Test user input");
        let b = sha256_first_8("Test user input");
        assert_eq!(a, b);
    }

    #[test]
    fn hash_events_stores_short_user_text_previews() {
        let text = "a".repeat(USER_TEXT_PREVIEW_CHARS + 20);
        let (hash_chain, user_texts, _kept_indices) = hash_events(&[text.clone()]);

        assert_eq!(hash_chain.len(), 1);
        assert_eq!(user_texts.len(), 1);
        assert_eq!(
            user_texts[0],
            format!(
                "{}...",
                text.chars()
                    .take(USER_TEXT_PREVIEW_CHARS)
                    .collect::<String>()
            )
        );
        assert!(user_texts[0].len() < text.len());
    }

    #[test]
    fn build_tree_single_root() {
        let files = vec![CachedFileData {
            session_key: "claude:a:path1".into(),
            source_path: "path1".into(),
            title: "Session A".into(),
            summary: None,
            last_active_at: None,
            project_dir: None,
            hash_chain: vec!["h1".into(), "h2".into()],
            user_texts: vec![],
            kept_indices: vec![],
            forked_from_id: None,
            uuid_chain: vec![],
        }];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].session_key, "claude:a:path1");
        assert_eq!(roots[0].depth, 0);
        assert!(roots[0].children.is_empty());
    }

    #[test]
    fn build_tree_linear_chain_via_forked_from_id() {
        let files = vec![
            CachedFileData {
                session_key: "claude:a:path1".into(),
                source_path: "path1".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "claude:b:path2".into(),
                source_path: "path2".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h3".into()],
                user_texts: vec![],
                // Without forked_from_id, sessions are always roots
                kept_indices: vec![],
            forked_from_id: Some("a".into()),
                uuid_chain: vec![],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].session_key, "claude:a:path1");
        assert_eq!(roots[0].children.len(), 1);
        assert_eq!(roots[0].children[0].session_key, "claude:b:path2");
        assert_eq!(roots[0].children[0].depth, 1);
        assert_eq!(roots[0].children[0].forked_at_user, 2);
    }

    #[test]
    fn build_tree_heuristic_same_provider() {
        // Two Claude sessions without forked_from_id.
        // B's hash chain has A's full chain as prefix → heuristic makes B a child of A.
        let files = vec![
            CachedFileData {
                session_key: "claude:a:path1".into(),
                source_path: "path1".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec!["hello".into(), "world".into()],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "claude:b:path2".into(),
                source_path: "path2".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h3".into()],
                user_texts: vec!["hello".into(), "world".into(), "new".into()],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1, "A is root, B is child via heuristic");
        assert_eq!(roots[0].session_key, "claude:a:path1");
        assert_eq!(roots[0].children.len(), 1);
        let child = &roots[0].children[0];
        assert_eq!(child.session_key, "claude:b:path2");
        assert_eq!(child.forked_at_user, 2, "B forks at user index 2 (its 3rd message)");
        assert_eq!(child.depth, 1);
    }

    #[test]
    fn build_tree_heuristic_different_providers_no_false_positive() {
        // Two sessions from different providers with same hash chain prefix.
        // Heuristic is same-provider only → both remain roots.
        let files = vec![
            CachedFileData {
                session_key: "claude:a:path1".into(),
                source_path: "path1".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "codex:b:path2".into(),
                source_path: "path2".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h3".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 2, "different providers → both roots despite hash overlap");
    }

    #[test]
    fn build_tree_fork_via_forked_from_id() {
        let files = vec![
            CachedFileData {
                session_key: "claude:a:path1".into(),
                source_path: "path1".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h3".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "claude:b:path2".into(),
                source_path: "path2".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h4".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: Some("a".into()),
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "claude:c:path3".into(),
                source_path: "path3".into(),
                title: "C".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h4".into(), "h5".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: Some("b".into()),
                uuid_chain: vec![],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].session_key, "claude:a:path1");
        assert_eq!(roots[0].children.len(), 1);
        // B is a child of A, forked at user 2
        let b = &roots[0].children[0];
        assert_eq!(b.session_key, "claude:b:path2");
        assert_eq!(b.forked_at_user, 2);
        assert_eq!(b.depth, 1);
        // C is a child of B, forked at user 3
        assert_eq!(b.children.len(), 1);
        assert_eq!(b.children[0].session_key, "claude:c:path3");
        assert_eq!(b.children[0].forked_at_user, 3);
    }

    #[test]
    fn build_tree_two_roots() {
        let files = vec![
            CachedFileData {
                session_key: "claude:a:path1".into(),
                source_path: "path1".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "claude:b:path2".into(),
                source_path: "path2".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h3".into(), "h4".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 2);
    }

    #[test]
    fn build_tree_fork_user_text_via_forked_from_id() {
        let files = vec![
            CachedFileData {
                session_key: "claude:a:path1".into(),
                source_path: "path1".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec!["hello".into(), "world".into()],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "claude:b:path2".into(),
                source_path: "path2".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h3".into()],
                user_texts: vec!["hello".into(), "different path".into()],
                kept_indices: vec![],
            forked_from_id: Some("a".into()),
                uuid_chain: vec![],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1);
        // A is root, B is child forked at user 1
        assert_eq!(roots[0].children.len(), 1);
        let child = &roots[0].children[0];
        assert_eq!(child.forked_at_user, 1);
        assert_eq!(child.fork_user_text.as_deref(), Some("different path"));
    }

    #[test]
    fn build_tree_forked_at_user_maps_through_kept_indices() {
        // Simulates: original user events = ["hello"(filtered), "real Q1", "real Q2"]
        // hash_chain only has 2 entries (greeting removed), kept_indices = [1, 2]
        // LCP of 1 should map to original index 2 (not 1).
        let files = vec![
            CachedFileData {
                session_key: "claude:a:path1".into(),
                source_path: "path1".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into()],
                user_texts: vec!["real Q1".into()],
                kept_indices: vec![1],
                forked_from_id: None,
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "claude:b:path2".into(),
                source_path: "path2".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec!["real Q1".into(), "real Q2".into()],
                kept_indices: vec![1, 2],
                forked_from_id: Some("a".into()),
                uuid_chain: vec![],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1);
        let child = &roots[0].children[0];
        assert_eq!(child.session_key, "claude:b:path2");
        // LCP = 1 (chain-space) → kept_indices[1] = 2 (original event index)
        assert_eq!(
            child.forked_at_user, 2,
            "forked_at_user should be original index 2, not chain index 1"
        );
        // fork_user_text still uses chain-space lookup
        assert_eq!(child.fork_user_text.as_deref(), Some("real Q2"));
    }

    #[test]
    fn compute_fork_tree_integration() {
        let _guard = ENV_LOCK.lock().expect("lock");

        let test_home = tempdir().expect("tempdir");
        let claude_dir = test_home.path().join(".claude");
        std::env::set_var("CLAUDE_CONFIG_DIR", &claude_dir);
        // Isolate from other tests that may have left SESSION_MANAGER_TEST_HOME set
        std::env::set_var("SESSION_MANAGER_TEST_HOME", test_home.path());

        // Create two sessions that share the first user event
        let projects = claude_dir.join("projects");
        fs::create_dir_all(&projects).expect("create dirs");

        // Session A: [user("hello"), user("world")]
        {
            let path = projects.join("a.jsonl");
            let mut f = fs::File::create(&path).expect("create");
            writeln!(f, "{{\"sessionId\":\"a\",\"cwd\":\"/tmp\"}}").expect("write");
            writeln!(
                f,
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hello\"}}]}}}}"
            )
            .expect("write");
            writeln!(
                f,
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"world\"}}]}}}}"
            )
            .expect("write");
        }

        // Session B: [user("hello"), user("different")]
        {
            let path = projects.join("b.jsonl");
            let mut f = fs::File::create(&path).expect("create");
            writeln!(f, "{{\"sessionId\":\"b\",\"cwd\":\"/tmp\"}}").expect("write");
            writeln!(
                f,
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"hello\"}}]}}}}"
            )
            .expect("write");
            writeln!(
                f,
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"different\"}}]}}}}"
            )
            .expect("write");
        }

        let registry = build_provider_registry();
        let result =
            compute_fork_tree(&registry, &session_manager::SessionScope::Active, None).expect("compute");
        assert_eq!(result.total_sessions, 2);
        assert!(!result.computed_from_cache);

        // No forked_from_id for Claude sessions.
        // A and B both start with "hello" which is filtered as a greeting,
        // leaving "world" vs "different" — no LCP match → both roots.
        assert_eq!(result.roots.len(), 2);
        assert_eq!(result.roots[0].children.len(), 0);
        assert_eq!(result.roots[1].children.len(), 0);

        // Second call re-scans but reuses cache for hash chain computation
        let result2 =
            compute_fork_tree(&registry, &session_manager::SessionScope::Active, None).expect("compute");
        assert_eq!(result2.total_sessions, 2);

        // get_fork_tree reads persisted cache and reports as cached
        let result3 = get_fork_tree().expect("get cached");
        assert!(result3.computed_from_cache);
        assert_eq!(result3.total_sessions, 2);

        // Clean up env
        std::env::remove_var("CLAUDE_CONFIG_DIR");
        std::env::remove_var("SESSION_MANAGER_TEST_HOME");
    }

    #[test]
    fn build_tree_forked_from_id_explicit_parent() {
        // Session A (root) has no forked_from_id
        // Session B has forked_from_id = "a" → B should be child of A
        let files = vec![
            CachedFileData {
                session_key: "codex:a:/tmp/a.jsonl".into(),
                source_path: "a.jsonl".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h3".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "codex:b:/tmp/b.jsonl".into(),
                source_path: "b.jsonl".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h4".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: Some("a".to_string()),
                uuid_chain: vec![],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].session_key, "codex:a:/tmp/a.jsonl");
        assert_eq!(roots[0].children.len(), 1);
        let b = &roots[0].children[0];
        assert_eq!(b.session_key, "codex:b:/tmp/b.jsonl");
        assert_eq!(b.forked_at_user, 2); // prefix len = 2 (h1, h2 match)
        assert_eq!(b.depth, 1);
    }

    #[test]
    fn build_tree_forked_from_id_chain() {
        // A → B → C chain, all using forked_from_id
        let files = vec![
            CachedFileData {
                session_key: "codex:a:/tmp/a.jsonl".into(),
                source_path: "a.jsonl".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "codex:b:/tmp/b.jsonl".into(),
                source_path: "b.jsonl".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h3".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: Some("a".to_string()),
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "codex:c:/tmp/c.jsonl".into(),
                source_path: "c.jsonl".into(),
                title: "C".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h3".into(), "h4".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: Some("b".to_string()),
                uuid_chain: vec![],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].session_key, "codex:a:/tmp/a.jsonl");
        assert_eq!(roots[0].children.len(), 1);
        let b = &roots[0].children[0];
        assert_eq!(b.session_key, "codex:b:/tmp/b.jsonl");
        assert_eq!(b.forked_at_user, 2);
        assert_eq!(b.depth, 1);
        assert_eq!(b.children.len(), 1);
        let c = &b.children[0];
        assert_eq!(c.session_key, "codex:c:/tmp/c.jsonl");
        assert_eq!(c.forked_at_user, 3);
        assert_eq!(c.depth, 2);
    }

    #[test]
    fn build_tree_forked_from_id_parent_longer_chain() {
        // Parent (A) has longer chain than child (B).
        // B has forked_from_id → should find A as parent.
        // Without forked_from_id support, B would become root and A a child of B.
        // Sorted by chain length: B(len=2) first, A(len=3) second.
        let files = vec![
            CachedFileData {
                session_key: "codex:a:/tmp/a.jsonl".into(),
                source_path: "a.jsonl".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h3".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec![],
            },
            CachedFileData {
                session_key: "codex:b:/tmp/b.jsonl".into(),
                source_path: "b.jsonl".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: Some("a".to_string()),
                uuid_chain: vec![],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].session_key, "codex:a:/tmp/a.jsonl");
        assert_eq!(roots[0].children.len(), 1);
        let b = &roots[0].children[0];
        assert_eq!(b.session_key, "codex:b:/tmp/b.jsonl");
        assert_eq!(b.forked_at_user, 2); // common prefix = 2
        assert_eq!(b.depth, 1);
    }

    #[test]
    fn build_tree_forked_from_id_orphan_becomes_root() {
        // Session has forked_from_id pointing to a session not in the file set
        let files = vec![CachedFileData {
            session_key: "codex:orphan:/tmp/o.jsonl".into(),
            source_path: "o.jsonl".into(),
            title: "Orphan".into(),
            summary: None,
            last_active_at: None,
            project_dir: None,
            hash_chain: vec!["h1".into(), "h2".into()],
            user_texts: vec![],
            kept_indices: vec![],
            forked_from_id: Some("nonexistent-parent".to_string()),
            uuid_chain: vec![],
        }];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].session_key, "codex:orphan:/tmp/o.jsonl");
        assert_eq!(roots[0].depth, 0);
    }

    /// Integration test using synthetic Codex session files.
    /// Verifies end-to-end: JSONL parsing → hash chain computation → fork tree construction.
    ///
    /// Synthetic fixtures at tests/fixtures/synthetic-codex/ encode this topology:
    ///   A (root, 4 events) → B (child, fork_at=2, 5 events)
    ///   C (root, 4 events) → D (child, fork_at=2, 3 events)
    ///                       → E (child, fork_at=3, 4 events)
    ///   F (root, 2 events)  → G (child, fork_at=1, 3 events)
    ///   H (root, 2 events)  → I (child, fork_at=1, 2 events)
    ///   K, L, M, N(orphan)  → standalone roots
    ///   S1, S2              → subagent (filtered out)
    #[test]
    fn synthetic_codex_fork_tree() {
        let _guard = ENV_LOCK.lock().expect("lock");

        let fixture_dir = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/synthetic-codex"
        ));
        assert!(
            fixture_dir.exists(),
            "synthetic-codex fixture dir not found — run gen-fixtures script"
        );

        let registry = build_provider_registry();

        // Parse all fixture files through the full pipeline
        let mut files: Vec<CachedFileData> = Vec::new();
        let mut total_files = 0u32;
        let mut subagent_count = 0u32;
        for entry in fs::read_dir(&fixture_dir).expect("read fixture dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                total_files += 1;
                match compute_file_data(&registry, &path) {
                    Ok(data) => files.push(data),
                    Err(_) => {
                        subagent_count += 1;
                    }
                }
            }
        }
        assert_eq!(total_files, 15, "expected 15 synthetic session files");
        assert_eq!(
            subagent_count, 2,
            "expected 2 subagent threads to be filtered"
        );
        assert_eq!(files.len(), 13, "expected 13 non-subagent sessions");

        let roots = build_tree(&files);

        // Build flat lookup: session_key → TreeNodeData
        fn collect_nodes<'a>(roots: &'a [TreeNodeData], out: &mut Vec<&'a TreeNodeData>) {
            for r in roots {
                out.push(r);
                collect_nodes(&r.children, out);
            }
        }
        let mut all_nodes = Vec::new();
        collect_nodes(&roots, &mut all_nodes);

        // All 13 non-subagent session IDs present
        let expected_ids = [
            "synth-a-0000",
            "synth-b-1111",
            "synth-c-2222",
            "synth-d-3333",
            "synth-e-4444",
            "synth-f-5555",
            "synth-g-6666",
            "synth-h-7777",
            "synth-i-8888",
            "synth-k-9999",
            "synth-l-aaaa",
            "synth-m-bbbb",
            "synth-n-cccc",
        ];
        for sid in &expected_ids {
            let found = all_nodes.iter().any(|n| n.session_key.contains(sid));
            assert!(found, "session {sid} not found in fork tree");
        }

        // Subagent sessions are NOT in the tree
        for sid in &["synth-s1-sub", "synth-s2-sub"] {
            let found = all_nodes.iter().any(|n| n.session_key.contains(sid));
            assert!(!found, "subagent session {sid} should be filtered out");
        }

        // A (root) → B (child, fork_at=2: first 2 user msgs are identical)
        {
            let parent = all_nodes
                .iter()
                .find(|n| n.session_key.contains("synth-a-0000"))
                .expect("parent A");
            assert_eq!(parent.depth, 0, "A should be root");
            let child = parent
                .children
                .iter()
                .find(|c| c.session_key.contains("synth-b-1111"));
            assert!(child.is_some(), "B should be child of A");
            if let Some(c) = child {
                assert_eq!(c.depth, 1);
                assert_eq!(
                    c.forked_at_user, 2,
                    "B should fork from A at user message index 2"
                );
            }
        }

        // C (root) → D (fork_at=2), E (fork_at=3)
        {
            let parent = all_nodes
                .iter()
                .find(|n| n.session_key.contains("synth-c-2222"))
                .expect("parent C");
            assert_eq!(parent.depth, 0, "C should be root");
            let child_d = parent
                .children
                .iter()
                .find(|c| c.session_key.contains("synth-d-3333"));
            let child_e = parent
                .children
                .iter()
                .find(|c| c.session_key.contains("synth-e-4444"));
            assert!(child_d.is_some(), "D should be child of C");
            assert!(child_e.is_some(), "E should be child of C");
            if let Some(c) = child_d {
                assert_eq!(c.depth, 1);
                assert_eq!(
                    c.forked_at_user, 2,
                    "D should fork from C at user message index 2"
                );
            }
            if let Some(c) = child_e {
                assert_eq!(c.depth, 1);
                assert_eq!(
                    c.forked_at_user, 3,
                    "E should fork from C at user message index 3"
                );
            }
        }

        // F (root) → G (fork_at=1)
        {
            let parent = all_nodes
                .iter()
                .find(|n| n.session_key.contains("synth-f-5555"))
                .expect("parent F");
            assert_eq!(parent.depth, 0, "F should be root");
            let child = parent
                .children
                .iter()
                .find(|c| c.session_key.contains("synth-g-6666"));
            assert!(child.is_some(), "G should be child of F");
            if let Some(c) = child {
                assert_eq!(c.depth, 1);
                assert_eq!(
                    c.forked_at_user, 1,
                    "G should fork from F at user message index 1"
                );
            }
        }

        // H (root) → I (fork_at=1)
        {
            let parent = all_nodes
                .iter()
                .find(|n| n.session_key.contains("synth-h-7777"))
                .expect("parent H");
            assert_eq!(parent.depth, 0, "H should be root");
            let child = parent
                .children
                .iter()
                .find(|c| c.session_key.contains("synth-i-8888"));
            assert!(child.is_some(), "I should be child of H");
            if let Some(c) = child {
                assert_eq!(c.depth, 1);
                assert_eq!(
                    c.forked_at_user, 1,
                    "I should fork from H at user message index 1"
                );
            }
        }

        // Remaining roots: K, L, M, N (orphan) — should have no parent
        let other_roots = [
            "synth-k-9999",
            "synth-l-aaaa",
            "synth-m-bbbb",
            "synth-n-cccc",
        ];
        {
            fn child_keys(roots: &[TreeNodeData]) -> Vec<String> {
                let mut keys = Vec::new();
                for r in roots {
                    for c in &r.children {
                        keys.push(c.session_key.clone());
                        keys.append(&mut child_keys(&c.children));
                    }
                }
                keys
            }
            let all_child_keys = child_keys(&roots);
            for sid in &other_roots {
                let has_parent = all_child_keys.iter().any(|k| k.contains(sid));
                assert!(!has_parent, "{sid} should be root but appears as a child");
            }
        }
    }

    // ── Path B: uuid chain LCP tests ───────────────────────────────────────────

    #[test]
    fn build_tree_uuid_chain_heuristic() {
        // Two Claude sessions (same provider), no forked_from_id.
        // B's uuid_chain has A's full chain as prefix -> B is child of A.
        let files = vec![
            CachedFileData {
                session_key: "claude:a:path1".into(),
                source_path: "path1".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec!["hello".into(), "world".into()],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec!["u1".into(), "u2".into()],
            },
            CachedFileData {
                session_key: "claude:b:path2".into(),
                source_path: "path2".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h3".into()],
                user_texts: vec!["hello".into(), "world".into(), "new".into()],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec!["u1".into(), "u2".into(), "u3".into()],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1, "A is root, B is child via uuid-chain LCP");
        assert_eq!(roots[0].session_key, "claude:a:path1");
        assert_eq!(roots[0].children.len(), 1);
        let child = &roots[0].children[0];
        assert_eq!(child.session_key, "claude:b:path2");
        assert_eq!(
            child.forked_at_user, 2,
            "B forks at user index 2 via uuid-chain LCP"
        );
        assert_eq!(child.depth, 1);
    }

    #[test]
    fn build_tree_uuid_chain_different_providers() {
        // Two sessions, different providers, same uuid_chain prefix.
        // Heuristic is same-provider only -> both remain roots.
        let files = vec![
            CachedFileData {
                session_key: "claude:a:path1".into(),
                source_path: "path1".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec!["u1".into(), "u2".into()],
            },
            CachedFileData {
                session_key: "gemini:b:path2".into(),
                source_path: "path2".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec!["u1".into(), "u2".into()],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(
            roots.len(),
            2,
            "different providers -> both roots despite uuid_chain overlap"
        );
    }

    #[test]
    fn build_tree_uuid_chain_two_roots() {
        // Two sessions, same provider, uuid_chain has no overlap.
        let files = vec![
            CachedFileData {
                session_key: "claude:a:path1".into(),
                source_path: "path1".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec!["u1".into(), "u2".into()],
            },
            CachedFileData {
                session_key: "claude:b:path2".into(),
                source_path: "path2".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h3".into(), "h4".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec!["u3".into(), "u4".into()],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 2);
    }

    // ── Path A with uuid chain fallback ────────────────────────────────────────

    #[test]
    fn build_tree_forked_from_id_with_uuid() {
        // Codex session A has no forked_from_id (root).
        // Codex session B has forked_from_id = Some("a") and uuid_chain.
        // fork_at should come from uuid_chain LCP (both sides have it),
        // not hash_chain LCP.
        let files = vec![
            CachedFileData {
                session_key: "codex:a:/tmp/a.jsonl".into(),
                source_path: "a.jsonl".into(),
                title: "A".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h3".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: None,
                uuid_chain: vec!["u1".into(), "u2".into(), "u3".into()],
            },
            CachedFileData {
                session_key: "codex:b:/tmp/b.jsonl".into(),
                source_path: "b.jsonl".into(),
                title: "B".into(),
                summary: None,
                last_active_at: None,
                project_dir: None,
                hash_chain: vec!["h1".into(), "h2".into(), "h4".into()],
                user_texts: vec![],
                kept_indices: vec![],
            forked_from_id: Some("a".to_string()),
                uuid_chain: vec!["u1".into(), "u2".into(), "b3".into(), "b4".into()],
            },
        ];

        let roots = build_tree(&files);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].session_key, "codex:a:/tmp/a.jsonl");
        assert_eq!(roots[0].children.len(), 1);
        let b = &roots[0].children[0];
        assert_eq!(b.session_key, "codex:b:/tmp/b.jsonl");
        assert_eq!(
            b.forked_at_user, 2,
            "fork_at from uuid_chain LCP (u1, u2), not hash_chain"
        );
        assert_eq!(b.depth, 1);
    }

    // ── E2E integration: synthetic Claude fixtures ─────────────────────────────

    /// Integration test using synthetic Claude session files.
    /// Verifies end-to-end: JSONL parsing -> uuid chain extraction -> fork tree construction.
    ///
    /// Synthetic fixtures at tests/fixtures/synthetic-claude/ encode this topology:
    ///   A (root, 4 events) -> B (child, fork_at=2, 4 events)
    ///                       -> C (child, fork_at=3, 4 events)
    ///   D (root, 3 events) -> E (child, fork_at=1, 3 events)
    ///   F, G, H             -> standalone roots
    ///   S1, S2              -> subagent (filtered out via agent- prefix)
    #[test]
    fn synthetic_claude_fork_tree() {
        let _guard = ENV_LOCK.lock().expect("lock");

        let fixture_dir = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/synthetic-claude"
        ));
        assert!(
            fixture_dir.exists(),
            "synthetic-claude fixture dir not found - run gen-claude-fixtures.py"
        );

        let registry = build_provider_registry();

        // Parse all fixture files through the full pipeline
        let mut files: Vec<CachedFileData> = Vec::new();
        let mut total_files = 0u32;
        let mut subagent_count = 0u32;
        for entry in fs::read_dir(&fixture_dir).expect("read fixture dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                total_files += 1;
                match compute_file_data(&registry, &path) {
                    Ok(data) => files.push(data),
                    Err(_) => {
                        subagent_count += 1;
                    }
                }
            }
        }
        assert_eq!(
            total_files, 10,
            "expected 10 synthetic claude session files"
        );
        assert_eq!(
            subagent_count, 2,
            "expected 2 subagent threads to be filtered"
        );
        assert_eq!(files.len(), 8, "expected 8 non-subagent sessions");

        let roots = build_tree(&files);

        // Build flat lookup: session_key -> TreeNodeData
        fn collect_nodes<'a>(roots: &'a [TreeNodeData], out: &mut Vec<&'a TreeNodeData>) {
            for r in roots {
                out.push(r);
                collect_nodes(&r.children, out);
            }
        }
        let mut all_nodes = Vec::new();
        collect_nodes(&roots, &mut all_nodes);

        // All 8 non-subagent session IDs present
        let expected_ids = [
            "synth-u-a",
            "synth-u-b",
            "synth-u-c",
            "synth-u-d",
            "synth-u-e",
            "synth-u-f",
            "synth-u-g",
            "synth-u-h",
        ];
        for sid in &expected_ids {
            let found = all_nodes.iter().any(|n| n.session_key.contains(sid));
            assert!(found, "session {sid} not found in fork tree");
        }

        // Subagent sessions are NOT in the tree
        for sid in &["synth-u-s1", "synth-u-s2"] {
            let found = all_nodes.iter().any(|n| n.session_key.contains(sid));
            assert!(!found, "subagent session {sid} should be filtered out");
        }

        // A (root) -> B (child, fork_at=2: first 2 uuids match)
        {
            let parent = all_nodes
                .iter()
                .find(|n| n.session_key.contains("synth-u-a"))
                .expect("parent A");
            assert_eq!(parent.depth, 0, "A should be root");
            let child = parent
                .children
                .iter()
                .find(|c| c.session_key.contains("synth-u-b"));
            assert!(child.is_some(), "B should be child of A");
            if let Some(c) = child {
                assert_eq!(c.depth, 1);
                assert_eq!(
                    c.forked_at_user, 2,
                    "B should fork from A at user message index 2"
                );
            }
        }

        // A (root) -> C (child, fork_at=3: first 3 uuids match)
        {
            let parent = all_nodes
                .iter()
                .find(|n| n.session_key.contains("synth-u-a"))
                .expect("parent A");
            let child = parent
                .children
                .iter()
                .find(|c| c.session_key.contains("synth-u-c"));
            assert!(child.is_some(), "C should be child of A");
            if let Some(c) = child {
                assert_eq!(c.depth, 1);
                assert_eq!(
                    c.forked_at_user, 3,
                    "C should fork from A at user message index 3"
                );
            }
        }

        // D (root) -> E (child, fork_at=1: first uuid matches)
        {
            let parent = all_nodes
                .iter()
                .find(|n| n.session_key.contains("synth-u-d"))
                .expect("parent D");
            assert_eq!(parent.depth, 0, "D should be root");
            let child = parent
                .children
                .iter()
                .find(|c| c.session_key.contains("synth-u-e"));
            assert!(child.is_some(), "E should be child of D");
            if let Some(c) = child {
                assert_eq!(c.depth, 1);
                assert_eq!(
                    c.forked_at_user, 1,
                    "E should fork from D at user message index 1"
                );
            }
        }

        // F, G, H are standalone roots (no children)
        for sid in &["synth-u-f", "synth-u-g", "synth-u-h"] {
            let node = all_nodes
                .iter()
                .find(|n| n.session_key.contains(sid))
                .expect("standalone root");
            assert_eq!(node.depth, 0, "{sid} should be root");
            assert!(node.children.is_empty(), "{sid} should have no children");
        }
    }
}
