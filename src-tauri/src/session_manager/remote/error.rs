//! Error type for the SSH remote-source connection layer.
//!
//! This module is deliberately free of russh types so the framing/cache
//! units (and any future consumer) can reason about failures without a
//! transport dependency. Transport errors are flattened into `Io` (or a
//! more specific variant where the upper layer must react, e.g.
//! `Disconnected` drives the one-reconnect retry in `RemoteSession`).

use std::fmt;

/// Failure of a remote-source operation.
///
/// Variants are chosen for upper-layer REACTION shape, not transport
/// taxonomy: `Disconnected` → retry once then surface "reconnect";
/// `AuthFailed` → surface credentials problem; `NotFound` → converge the
/// session list on next scan; `ExecUnavailable` → restricted-shell
/// diagnosis (no SFTP degradation in v1, see ADR 0007 discipline).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteError {
    /// The transport died mid-operation (connection reset, idle timeout,
    /// server gone). `RemoteSession` retries once internally; a second
    /// occurrence must surface as "reconnect" in the UI, never as
    /// "directory is empty".
    Disconnected,
    /// Authentication failed with every configured method (agent
    /// identities and key file). Check `auth` in the ssh source settings.
    AuthFailed(String),
    /// The remote file no longer exists (deleted after the scan that
    /// produced the request). Callers should drop/invalidate the cached
    /// entry and let the next scan converge.
    NotFound(String),
    /// The exec channel is unusable (e.g. a restricted shell refuses to
    /// run the batch script). v1 performs no SFTP fallback — the batch
    /// metadata path has no cache/batch-free substitute (ADR 0007).
    ExecUnavailable(String),
    /// The server host key is not in the user's known_hosts. The user
    /// must connect once with the system `ssh` client to record it —
    /// this layer never auto-learns host keys (TOFU is rejected for a
    /// product surface, unlike the read-only spike).
    HostKeyUnknown { host: String, port: u16 },
    /// The server host key matches no entry but a DIFFERENT key was
    /// previously recorded for this host (possible MITM). Same remedy as
    /// `HostKeyUnknown` but the user should investigate before trusting.
    HostKeyChanged { host: String, port: u16 },
    /// The `sshConfig` auth mode's alias could not be resolved into a
    /// usable connection (ADR 0010): missing/unparseable
    /// `~/.ssh/config`, no matching `Host` block, or a `ProxyJump`
    /// entry — the russh stack has no jump-host dialing yet, so that
    /// case fails explicitly instead of dialing the target directly.
    /// Remedy: fix the `Host` block in `~/.ssh/config` or switch the
    /// source's auth mode.
    SshConfigAlias(String),
    /// Any other transport/local IO failure, with context.
    Io(String),
}

impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoteError::Disconnected => {
                write!(f, "remote connection lost mid-operation")
            }
            RemoteError::AuthFailed(detail) => {
                write!(
                    f,
                    "SSH authentication failed (agent and key file): {detail}"
                )
            }
            RemoteError::NotFound(path) => {
                write!(f, "remote file not found: {path}")
            }
            RemoteError::ExecUnavailable(detail) => {
                write!(
                    f,
                    "remote exec channel unavailable (restricted shell?): {detail}"
                )
            }
            RemoteError::HostKeyUnknown { host, port } => {
                write!(
                    f,
                    "unknown SSH host key for {host}:{port} — connect once with the system \
                     `ssh` client to record it in known_hosts, then retry"
                )
            }
            RemoteError::HostKeyChanged { host, port } => {
                write!(
                    f,
                    "SSH host key for {host}:{port} changed since it was recorded in \
                     known_hosts — verify the server before retrying"
                )
            }
            RemoteError::SshConfigAlias(detail) => {
                write!(f, "ssh config alias resolution failed: {detail}")
            }
            RemoteError::Io(detail) => write!(f, "remote IO error: {detail}"),
        }
    }
}

impl std::error::Error for RemoteError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_key_unknown_display_is_actionable() {
        let msg = RemoteError::HostKeyUnknown {
            host: "192.0.2.10".to_string(),
            port: 22,
        }
        .to_string();
        // The message must tell the user the concrete remedy, not just
        // "key check failed".
        assert!(msg.contains("ssh"), "remedy mentions the ssh client: {msg}");
        assert!(
            msg.contains("known_hosts"),
            "remedy mentions known_hosts: {msg}"
        );
        assert!(msg.contains("192.0.2.10:22"));
    }

    #[test]
    fn host_key_changed_distinguishes_itself_from_unknown() {
        let changed = RemoteError::HostKeyChanged {
            host: "h".to_string(),
            port: 22,
        }
        .to_string();
        assert!(changed.contains("changed"), "{changed}");
    }

    #[test]
    fn variants_are_distinct_for_match_based_routing() {
        let variants = [
            RemoteError::Disconnected,
            RemoteError::AuthFailed("nope".to_string()),
            RemoteError::NotFound("p".to_string()),
            RemoteError::ExecUnavailable("restricted".to_string()),
            RemoteError::HostKeyUnknown {
                host: "h".to_string(),
                port: 22,
            },
            RemoteError::HostKeyChanged {
                host: "h".to_string(),
                port: 22,
            },
            RemoteError::SshConfigAlias("no Host block".to_string()),
            RemoteError::Io("boom".to_string()),
        ];
        for (i, a) in variants.iter().enumerate() {
            for (j, b) in variants.iter().enumerate() {
                assert_eq!(i == j, *a == *b, "variant {i} vs {j} collided");
            }
        }
    }
}
