pub mod metadata;
pub mod providers;

mod messages;
mod operations;
mod scan;
mod types;

use std::path::Path;
use std::sync::Arc;

use self::providers::ProviderRegistry;

/// Parse session metadata from a session file path.
/// Dispatches to the appropriate provider's `parse_session` via the registry.
#[allow(dead_code)]
pub fn parse_session_meta(registry: &ProviderRegistry, path: &Path) -> Option<SessionMeta> {
    for provider in registry.all() {
        if let Some(meta) = provider.parse_session(path) {
            return Some(meta);
        }
    }
    None
}

// Re-export public types and functions
pub use types::{
    CumulativeTokenUsage, DeleteSessionOutcome, DeleteSessionRequest, SessionDetail,
    SessionMessage, SessionMeta, SessionScope, TokenUsage, ToolCallInfo,
};

// Registry-aware re-exports: these functions now require a registry reference.
// Callers (commands) receive the registry from Tauri managed state and pass it through.
pub use messages::{load_messages, load_session_detail};
pub use operations::{
    archive_session, archive_sessions, delete_session, delete_sessions, restore_session,
    restore_sessions,
};
pub use scan::scan_sessions_with_scope;

/// Build and return the provider registry with all built-in providers registered.
/// Called once during Tauri setup.
pub fn build_provider_registry() -> Arc<ProviderRegistry> {
    let mut registry = ProviderRegistry::new();
    registry.register(Box::new(providers::claude::ClaudeProvider));
    registry.register(Box::new(providers::codex::CodexProvider));
    registry.register(Box::new(providers::gemini::GeminiProvider));
    registry.register(Box::new(providers::hermes::HermesProvider));
    registry.register(Box::new(providers::openclaw::OpenClawProvider));
    registry.register(Box::new(providers::opencode::OpenCodeProvider));
    registry.register(Box::new(providers::qoder::QoderProvider));
    Arc::new(registry)
}

#[cfg(test)]
mod tests {
    use super::messages::extract_qa_pairs;
    use super::operations::{collect_session_outcomes, delete_session_with_roots};
    use super::*;
    use crate::config::TEST_ENV_LOCK;
    use std::path::Path;
    use tempfile::tempdir;

    // Use the global shared lock to prevent parallel tests from racing on env vars.
    static ENV_LOCK: &std::sync::Mutex<()> = &TEST_ENV_LOCK;

    fn test_registry() -> Arc<ProviderRegistry> {
        build_provider_registry()
    }

    fn write_claude_session(path: &Path, session_id: &str) {
        std::fs::write(
            path,
            format!(
                "{{\"sessionId\":\"{session_id}\",\"cwd\":\"/tmp/project\",\"timestamp\":\"2026-03-06T10:00:00Z\"}}\n\
                 {{\"message\":{{\"role\":\"user\",\"content\":\"hello\"}},\"timestamp\":\"2026-03-06T10:01:00Z\"}}\n",
            ),
        )
        .expect("write source");
    }

    fn message(role: &str, content: &str) -> SessionMessage {
        SessionMessage {
            role: role.to_string(),
            content: content.to_string(),
            ts: None,
            usage: None,
            cumulative_usage: None,
            tool_calls: None,
            tool_result: None,
        }
    }

    fn assistant_with_cumulative(content: &str, total_tokens: u64) -> SessionMessage {
        SessionMessage {
            role: "assistant".to_string(),
            content: content.to_string(),
            ts: None,
            usage: None,
            cumulative_usage: Some(CumulativeTokenUsage {
                input_tokens: total_tokens,
                output_tokens: 0,
                total_tokens,
            }),
            tool_calls: None,
            tool_result: None,
        }
    }

    #[test]
    fn qa_pairs_single_user_message_returns_empty() {
        let pairs = extract_qa_pairs(&[message("user", "u1")]);
        assert!(pairs.is_empty());
    }

    #[test]
    fn qa_pairs_pair_user_with_following_assistant() {
        let msgs = vec![
            message("user", "u1"),
            message("assistant", "a1"),
            message("user", "u2"),
            message("assistant", "a2"),
        ];
        let pairs = extract_qa_pairs(&msgs);

        assert_eq!(pairs.len(), 2);
        assert_eq!(msgs[pairs[0].question_idx].content, "u1");
        assert_eq!(msgs[pairs[0].answer_idx].content, "a1");
        assert_eq!(msgs[pairs[1].question_idx].content, "u2");
        assert_eq!(msgs[pairs[1].answer_idx].content, "a2");
    }

    #[test]
    fn qa_pairs_use_last_assistant_before_next_user() {
        let msgs = vec![
            message("user", "u1"),
            message("assistant", "a1"),
            message("assistant", "a2"),
            message("user", "u2"),
            message("assistant", "a3"),
        ];
        let pairs = extract_qa_pairs(&msgs);

        assert_eq!(pairs.len(), 2);
        assert_eq!(msgs[pairs[0].question_idx].content, "u1");
        assert_eq!(msgs[pairs[0].answer_idx].content, "a2");
        assert_eq!(msgs[pairs[1].question_idx].content, "u2");
        assert_eq!(msgs[pairs[1].answer_idx].content, "a3");
    }

    #[test]
    fn qa_pairs_ignore_non_conversation_roles() {
        let msgs = vec![
            message("user", "u1"),
            message("system", "system"),
            message("assistant", "a1"),
            message("tool", "tool output"),
            message("unknown", "unknown"),
        ];
        let pairs = extract_qa_pairs(&msgs);

        assert_eq!(pairs.len(), 1);
        assert_eq!(msgs[pairs[0].question_idx].content, "u1");
        assert_eq!(msgs[pairs[0].answer_idx].content, "a1");
    }

    #[test]
    fn qa_pairs_emit_final_answer() {
        let msgs = vec![message("user", "u1"), message("assistant", "a1")];
        let pairs = extract_qa_pairs(&msgs);

        assert_eq!(pairs.len(), 1);
        assert_eq!(msgs[pairs[0].question_idx].content, "u1");
        assert_eq!(msgs[pairs[0].answer_idx].content, "a1");
    }

    #[test]
    fn qa_pairs_do_not_pair_consecutive_users_with_previous_answer() {
        let msgs = vec![
            message("user", "u1"),
            message("assistant", "a1"),
            message("user", "u2"),
            message("user", "u3"),
            message("assistant", "a3"),
        ];
        let pairs = extract_qa_pairs(&msgs);

        assert_eq!(pairs.len(), 2);
        assert_eq!(msgs[pairs[0].question_idx].content, "u1");
        assert_eq!(msgs[pairs[0].answer_idx].content, "a1");
        assert_eq!(msgs[pairs[1].question_idx].content, "u3");
        assert_eq!(msgs[pairs[1].answer_idx].content, "a3");
    }

    #[test]
    fn qa_pairs_preserve_answer_cumulative_usage() {
        let msgs = vec![
            message("user", "u1"),
            message("assistant", "a1"),
            assistant_with_cumulative("a2", 120),
            message("user", "u2"),
            assistant_with_cumulative("a3", 150),
        ];
        let pairs = extract_qa_pairs(&msgs);

        assert_eq!(pairs.len(), 2);
        assert_eq!(msgs[pairs[0].answer_idx].content, "a2");
        assert_eq!(
            msgs[pairs[0].answer_idx]
                .cumulative_usage
                .map(|usage| usage.total_tokens),
            Some(120)
        );
        assert_eq!(
            msgs[pairs[1].answer_idx]
                .cumulative_usage
                .map(|usage| usage.total_tokens),
            Some(150)
        );
    }

    #[test]
    fn token_usage_total_includes_cache_tokens() {
        let usage = TokenUsage {
            input_tokens: 10,
            cache_creation_input_tokens: 20,
            cache_read_input_tokens: 30,
            output_tokens: 40,
        };
        let mut cumulative = CumulativeTokenUsage::default();

        cumulative.add_usage(usage);

        assert_eq!(usage.input_total(), 60);
        assert_eq!(usage.total(), 100);
        assert_eq!(cumulative.input_tokens, 60);
        assert_eq!(cumulative.output_tokens, 40);
        assert_eq!(cumulative.total_tokens, 100);
    }

    #[test]
    fn load_session_detail_omits_raw_content_when_messages_exist() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"lastPrompt\":\"fallback\"}\n",
                "{\"message\":{\"role\":\"user\",\"content\":\"hello\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
        .expect("write");

        let registry = test_registry();
        let detail =
            load_session_detail(&registry, "claude", &path.to_string_lossy()).expect("load detail");
        assert_eq!(detail.messages.len(), 1);
        assert_eq!(detail.raw_content, None);
    }

    #[test]
    fn load_session_detail_includes_raw_content_when_messages_are_empty() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"type\":\"last-prompt\",\"lastPrompt\":\"explain how fork detection works\"}\n",
                "{\"type\":\"ai-title\",\"aiTitle\":\"Fork tree architecture\"}\n",
            ),
        )
        .expect("write");

        let registry = test_registry();
        let detail =
            load_session_detail(&registry, "claude", &path.to_string_lossy()).expect("load detail");
        assert!(detail.messages.is_empty());
        assert_eq!(
            detail.raw_content.as_deref(),
            Some("explain how fork detection works\n\nFork tree architecture")
        );
    }

    #[test]
    fn accepts_source_path_under_allowed_provider_root() {
        let root = tempdir().expect("root");
        let source = root.path().join("session.jsonl");
        write_claude_session(&source, "session-1");

        let registry = test_registry();
        let deleted = delete_session_with_roots(
            &registry,
            "claude",
            "session-1",
            &source,
            &[root.path().to_path_buf()],
        )
        .expect("delete session");

        assert!(deleted);
        assert!(!source.exists());
    }

    #[test]
    fn rejects_source_path_outside_provider_root() {
        let root = tempdir().expect("root");
        let outside = tempdir().expect("outside");
        let source = outside.path().join("session.jsonl");
        write_claude_session(&source, "session-1");

        let registry = test_registry();
        let err = delete_session_with_roots(
            &registry,
            "claude",
            "session-1",
            &source,
            &[root.path().to_path_buf()],
        )
        .expect_err("outside root should be rejected");

        assert!(err.contains("outside provider roots"));
    }

    #[test]
    fn rejects_missing_source_path() {
        let root = tempdir().expect("root");
        let missing = root.path().join("missing.jsonl");

        let registry = test_registry();
        let err = delete_session_with_roots(
            &registry,
            "claude",
            "session-1",
            &missing,
            &[root.path().to_path_buf()],
        )
        .expect_err("missing source should fail");

        assert!(err.contains("session source not found"));
    }

    #[test]
    fn batch_delete_collects_successes_and_failures_in_order() {
        let requests = vec![
            DeleteSessionRequest {
                provider_id: "claude".to_string(),
                session_id: "s1".to_string(),
                source_path: "/tmp/s1".to_string(),
            },
            DeleteSessionRequest {
                provider_id: "claude".to_string(),
                session_id: "s2".to_string(),
                source_path: "/tmp/s2".to_string(),
            },
        ];

        let outcomes = collect_session_outcomes(&requests, "Session was not deleted", |request| {
            if request.session_id == "s1" {
                Ok(true)
            } else {
                Err("boom".to_string())
            }
        });

        assert_eq!(outcomes.len(), 2);
        assert!(outcomes[0].success);
        assert_eq!(outcomes[0].error, None);
        assert!(!outcomes[1].success);
        assert_eq!(outcomes[1].error.as_deref(), Some("boom"));
    }

    #[test]
    fn delete_session_sends_to_system_trash() {
        let _guard = ENV_LOCK.lock().expect("lock");

        let test_home = tempdir().expect("tempdir");
        std::env::set_var("SESSION_MANAGER_TEST_HOME", test_home.path());

        // Create session in ~/.claude/projects/my-folder/session.jsonl
        let claude_dir = test_home.path().join(".claude");
        let source = claude_dir
            .join("projects")
            .join("my-folder")
            .join("abc-session.jsonl");
        std::fs::create_dir_all(source.parent().unwrap()).expect("create dir");
        write_claude_session(&source, "session-1");

        let registry = test_registry();
        let root = claude_dir.join("projects");
        delete_session_with_roots(&registry, "claude", "session-1", &source, &[root])
            .expect("delete session");

        assert!(!source.exists(), "original session should be gone");

        std::env::remove_var("SESSION_MANAGER_TEST_HOME");
    }

    #[test]
    fn delete_session_trashes_sidecar_directory_too() {
        let _guard = ENV_LOCK.lock().expect("lock");

        let test_home = tempdir().expect("tempdir");
        std::env::set_var("SESSION_MANAGER_TEST_HOME", test_home.path());

        let claude_dir = test_home.path().join(".claude");
        let source = claude_dir
            .join("projects")
            .join("my-folder")
            .join("abc-session.jsonl");
        std::fs::create_dir_all(source.parent().unwrap()).expect("create dir");
        write_claude_session(&source, "session-1");

        let sidecar = claude_dir
            .join("projects")
            .join("my-folder")
            .join("abc-session");
        std::fs::create_dir_all(&sidecar).expect("create sidecar");
        std::fs::write(sidecar.join("agent-1.jsonl"), "{}").expect("write sidecar");

        let registry = test_registry();
        let root = claude_dir.join("projects");
        delete_session_with_roots(&registry, "claude", "session-1", &source, &[root])
            .expect("delete session");

        assert!(!source.exists(), "original session file should be gone");
        assert!(!sidecar.exists(), "sidecar directory should be gone");

        std::env::remove_var("SESSION_MANAGER_TEST_HOME");
    }

    #[test]
    fn delete_session_rejects_id_mismatch() {
        let _guard = ENV_LOCK.lock().expect("lock");

        let test_home = tempdir().expect("tempdir");
        std::env::set_var("SESSION_MANAGER_TEST_HOME", test_home.path());

        let claude_dir = test_home.path().join(".claude");
        let source = claude_dir
            .join("projects")
            .join("my-folder")
            .join("abc-session.jsonl");
        std::fs::create_dir_all(source.parent().unwrap()).expect("create dir");
        write_claude_session(&source, "session-1");

        let registry = test_registry();
        let root = claude_dir.join("projects");
        let result = delete_session_with_roots(&registry, "claude", "wrong-id", &source, &[root]);

        assert!(result.is_err(), "should reject wrong session id");
        let err = result.unwrap_err();
        assert!(
            err.contains("session ID mismatch") || err.contains("ID mismatch"),
            "error should mention ID mismatch: {err}"
        );

        assert!(source.exists(), "session file should survive on mismatch");

        std::env::remove_var("SESSION_MANAGER_TEST_HOME");
    }

    #[test]
    fn parse_session_meta_works_with_registry() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("session-abc.jsonl");
        std::fs::write(
            &path,
            concat!(
                "{\"sessionId\":\"session-abc\",\"cwd\":\"/tmp\",\"timestamp\":\"2026-03-06T10:00:00Z\"}\n",
                "{\"message\":{\"role\":\"user\",\"content\":\"hello world\"},\"timestamp\":\"2026-03-06T10:01:00Z\"}\n",
            ),
        )
        .expect("write");

        let registry = test_registry();
        let meta = parse_session_meta(&registry, &path).expect("parse");
        assert_eq!(meta.session_id, "session-abc");
    }
}
