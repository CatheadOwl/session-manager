//! G2 real-host E2E acceptance for the remote v1 source (reusable).
//!
//! Runs the planner workunit's G2 gate end-to-end against a real SSH host.
//! Skipped unless `REMOTE_E2E_HOST` is set:
//!
//! ```text
//! REMOTE_E2E_HOST=139.224.165.11 \
//! REMOTE_E2E_PORT=22 \
//! REMOTE_E2E_USER=admin \
//! REMOTE_E2E_KEY=~/.ssh/id_ed25519 \
//! REMOTE_E2E_SOURCE_ID=ali \
//! cargo test --offline remote_e2e -- --ignored --nocapture
//! ```
//!
//! The scan roots are DERIVED from each provider's `roots()`
//! (home-prefix strip, ADR 0008 修订 1) — there is no root/provider
//! env knob anymore; the host simply must have at least one populated
//! standard provider root.
//!
//! When direct outbound from a freshly built test binary is firewalled
//! (unsigned exe), run through an SSH local forward and point HOST/PORT at
//! it (known_hosts must cover that entry):
//! `ssh -L 2222:127.0.0.1:22 -N <alias>` + HOST=127.0.0.1 PORT=2222.
//!
//! G2 checklist (workunit 20260910-1145):
//! 1. list: remote scan produces Remote-locator SessionMeta
//! 2. open: message load via the cache bridge; second open is a cache hit
//!    (asserted structurally: same local path, freshness sidecar untouched)
//! 3. write entries disabled: lifecycle entries reject Remote handles
//! 4. local perf: local-only scan timing reported (report-only artifact)

#![cfg(test)]

use std::time::Instant;

use crate::session_manager::operations;
use crate::session_manager::providers::ProviderRegistry;
use crate::session_manager::remote::{resolve_remote_to_local, RemoteSessionPool};
use crate::session_manager::settings::{SourceAuth, SshSource};
use crate::session_manager::types::{SessionHandle, SessionLocator, SessionScope};
use crate::session_manager::{build_provider_registry, scan_sessions_with_scope};

fn e2e_config() -> Option<SshSource> {
    let host = std::env::var("REMOTE_E2E_HOST").ok()?;
    let key_path = std::env::var("REMOTE_E2E_KEY").unwrap_or_else(|_| {
        format!(
            "{}/.ssh/id_ed25519",
            std::env::var("USERPROFILE").unwrap_or_default()
        )
    });
    Some(SshSource {
        id: std::env::var("REMOTE_E2E_SOURCE_ID").unwrap_or_else(|_| "e2e".into()),
        label: None,
        host,
        port: std::env::var("REMOTE_E2E_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(22),
        user: std::env::var("REMOTE_E2E_USER").unwrap_or_else(|_| "admin".into()),
        auth: SourceAuth::Key {
            key_path: shellexpand_tilde(&key_path),
        },
        enabled: true,
        extra: Default::default(),
    })
}

fn shellexpand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
            return format!("{home}/{rest}");
        }
    }
    p.to_string()
}

#[tokio::test]
#[ignore = "needs a real SSH host (see module docs)"]
async fn g2_remote_v1_end_to_end() {
    // Minimal stderr logger so probe/warn diagnostics are visible under
    // --nocapture (the lib's log output is otherwise dropped in tests).
    struct E2eLogger;
    impl log::Log for E2eLogger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            eprintln!("[log:{}] {}", record.level(), record.args());
        }
        fn flush(&self) {}
    }
    static LOGGER: E2eLogger = E2eLogger;
    let _ = log::set_logger(&LOGGER).map(|()| log::set_max_level(log::LevelFilter::Debug));

    let source = e2e_config().expect(
        "REMOTE_E2E_HOST not set — this E2E needs a real SSH host; \
         see the module docs for the full env set",
    );
    let sid = source.id.clone();
    let registry = build_provider_registry();
    let pool = RemoteSessionPool::new();
    let sources = vec![source.clone()];

    // ---- G2.1 list: remote scan produces Remote-locator SessionMeta ----
    let state = crate::session_manager::remote::RemoteScanState::new();
    let session = state
        .pool
        .get(&source)
        .await
        .expect("connect + auth + known_hosts");
    let registry_arc = registry.clone();
    let t = Instant::now();
    let result = state
        .scan_source(&registry_arc, session, &source, &SessionScope::Active)
        .await;
    let outcome_sessions = result.sessions.clone();
    println!(
        "[g2.1] scan: {} sessions in {} ms (from_cache={})",
        outcome_sessions.len(),
        t.elapsed().as_millis(),
        result.from_cache
    );
    assert!(
        !result.from_cache,
        "first scan must hit the network, not the cache"
    );
    assert!(
        !outcome_sessions.is_empty(),
        "remote machine has no sessions in its standard provider roots — pick a populated host"
    );
    for meta in &outcome_sessions {
        assert!(
            matches!(meta.locator.as_ref(), Some(SessionLocator::Remote { source_id, .. }) if source_id == &sid),
            "every remote session must carry a Remote locator anchored to the source id"
        );
    }

    // ---- G2.2 open: message load through the cache bridge ----
    let target = &outcome_sessions[0];
    let handle = remote_handle(&sid, target);

    let t = Instant::now();
    let first = resolve_remote_to_local(&sources, &pool, &handle)
        .await
        .expect("first resolve")
        .expect("remote handle must resolve");
    println!(
        "[g2.2] first resolve: {} ms (network fetch)",
        t.elapsed().as_millis()
    );

    let messages = load_messages(&registry, &first);
    println!("[g2.2] loaded {} messages", messages.len());

    // Cache hit: same local path and the freshness sidecar is not rewritten.
    let local_path = first
        .locator
        .file_path()
        .expect("resolved handle is file-backed")
        .to_string();
    let sidecar = std::path::PathBuf::from(format!("{local_path}.meta.json"));
    let sidecar_before = std::fs::metadata(&sidecar)
        .expect("cache meta sidecar exists after first fetch")
        .modified()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let t = Instant::now();
    let second = resolve_remote_to_local(&sources, &pool, &handle)
        .await
        .expect("second resolve")
        .expect("remote handle must resolve again");
    let second_path = second.locator.file_path().unwrap().to_string();
    println!(
        "[g2.2] second resolve: {} ms (cache hit)",
        t.elapsed().as_millis()
    );
    assert_eq!(
        local_path, second_path,
        "cache hit returns the same local copy"
    );
    assert_eq!(
        sidecar_before,
        std::fs::metadata(&sidecar)
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(sidecar_before),
        "cache hit must not rewrite the freshness sidecar"
    );

    // ---- G2.3 write entries disabled ----
    let err = operations::delete_session_for_handle(&registry, &handle)
        .expect_err("delete must reject a Remote handle");
    assert!(
        err.contains("Remote"),
        "delete rejection must mention remote, got: {err}"
    );
    assert!(operations::archive_session_for_handle(&registry, &handle).is_err());
    assert!(operations::restore_session_for_handle(&registry, &handle).is_err());
    println!("[g2.3] lifecycle entries rejected: delete/archive/restore");

    // ---- G2.4 local perf: report-only artifact ----
    let t = Instant::now();
    let local = scan_sessions_with_scope(&registry, &SessionScope::Active, &[]);
    println!(
        "[g2.4] local scan baseline: {} sessions in {} ms (record as the regression artifact)",
        local.len(),
        t.elapsed().as_millis()
    );
    assert!(
        !local.is_empty(),
        "local machine should list its own sessions"
    );
}

fn remote_handle(
    source_id: &str,
    meta: &crate::session_manager::types::SessionMeta,
) -> SessionHandle {
    SessionHandle {
        provider_id: meta.provider_id.clone(),
        session_id: meta.session_id.clone(),
        locator: SessionLocator::Remote {
            source_id: source_id.to_string(),
            path: meta
                .locator
                .as_ref()
                .map(|l| l.display_source_path().to_string())
                .unwrap_or_default(),
        },
    }
}

fn load_messages(
    registry: &ProviderRegistry,
    handle: &SessionHandle,
) -> Vec<crate::session_manager::types::SessionMessage> {
    let provider = registry.get(&handle.provider_id).expect("provider");
    provider
        .load_messages_for_handle(handle)
        .expect("message load via resolved local copy")
}

/// ADR 0010 sshConfig-mode E2E (G2.1 scope): when `REMOTE_E2E_ALIAS` is
/// set, build the source with `auth: { mode: "sshConfig", alias }` —
/// host/user/port/key all come from the real `~/.ssh/config` Host
/// block at connect time — and run the list gate (connect + scan
/// produces Remote-locator SessionMeta).
///
/// ```text
/// REMOTE_E2E_ALIAS=ali \
/// REMOTE_E2E_SOURCE_ID=ali \
/// cargo test --offline remote_e2e_ssh_config_alias -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "needs a real ssh config alias (REMOTE_E2E_ALIAS) to a populated host"]
async fn remote_e2e_ssh_config_alias() {
    let alias = std::env::var("REMOTE_E2E_ALIAS").expect(
        "REMOTE_E2E_ALIAS not set — point it at a Host alias in ~/.ssh/config \
         whose machine has populated provider roots",
    );
    let source = SshSource {
        id: std::env::var("REMOTE_E2E_SOURCE_ID").unwrap_or_else(|_| "e2e-alias".into()),
        label: None,
        // Placeholder fields: the sshConfig auth mode overrides all
        // three from the resolved Host block at connect time.
        host: alias.clone(),
        port: 22,
        user: String::new(),
        auth: SourceAuth::SshConfig {
            alias: alias.clone(),
        },
        enabled: true,
        extra: Default::default(),
    };
    let sid = source.id.clone();
    let registry = build_provider_registry();

    // G2.1 list: connect resolves the alias, auth rides the agent pass
    // (or the Host block's IdentityFile, ~-expanded).
    let state = crate::session_manager::remote::RemoteScanState::new();
    let session = state
        .pool
        .get(&source)
        .await
        .expect("connect + auth via alias");
    let result = state
        .scan_source(&registry, session, &source, &SessionScope::Active)
        .await;
    println!(
        "[e2e-alias] scan: {} sessions (from_cache={})",
        result.sessions.len(),
        result.from_cache
    );
    assert!(!result.from_cache, "first scan must hit the network");
    assert!(
        !result.sessions.is_empty(),
        "aliased machine has no sessions in its standard provider roots"
    );
    for meta in &result.sessions {
        assert!(
            matches!(meta.locator.as_ref(), Some(SessionLocator::Remote { source_id, .. }) if source_id == &sid),
            "every remote session must carry a Remote locator anchored to the source id"
        );
    }
}
