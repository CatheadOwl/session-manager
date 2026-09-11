//! ssh-config alias resolution: expand the `sshConfig` auth
//! mode's `alias` against the user's `~/.ssh/config` into concrete
//! connection parameters.
//!
//! This is a LIVE reference, not a snapshot import: the config file is
//! re-read at every connect, so edits to `~/.ssh/config` are picked up
//! automatically (single source of truth, same semantics as VS Code
//! Remote-SSH). The heavy lifting — Host pattern matching (wildcards,
//! negation), first-obtained-value-wins parameter resolution, and
//! `Include` directives (resolved relative to `~/.ssh`, `~`-aware,
//! glob-supported) — is delegated to the `ssh2-config` crate; hand-
//! rolling those rules is a known trap.
//!
//! v1 boundary: an alias that resolves to a
//! `ProxyJump` fails loudly with "not supported yet" — the russh stack
//! has no jump-host dialing, and silently connecting to the jump target
//! directly would be wrong. Unknown aliases fail with an actionable
//! message pointing at `~/.ssh/config`.

use std::collections::HashSet;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use serde::Serialize;
use ssh2_config::{ParseRule, SshConfig};

use super::error::RemoteError;

/// One fully-resolved `Host` block from `~/.ssh/config`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAlias {
    /// HostName if the block set one, otherwise the alias itself (the
    /// OpenSSH default: `Host ali` + no HostName connects to "ali").
    pub host: String,
    /// `User` from the block, when present.
    pub user: Option<String>,
    /// `Port` from the block, when present.
    pub port: Option<u16>,
    /// First `IdentityFile` from the block, when present (ssh semantics:
    /// multiple files are tried in order; v1 surfaces the first as the
    /// key fallback after the agent pass).
    pub identity_file: Option<PathBuf>,
}

/// Resolve `alias` against the user's real `~/.ssh/config`.
///
/// Fails with an actionable [`RemoteError::SshConfigAlias`] when the
/// file is missing/unparseable, no `Host` block matches the alias, or
/// the matched block uses `ProxyJump`.
pub fn resolve_alias(alias: &str) -> Result<ResolvedAlias, RemoteError> {
    let path = dirs::home_dir()
        .ok_or_else(|| {
            RemoteError::SshConfigAlias("cannot determine the user home directory".to_string())
        })?
        .join(".ssh")
        .join("config");
    resolve_alias_in(&path, alias)
}

/// Test seam: same resolution against an explicit config file path
/// (temp-file backed unit tests; otherwise identical semantics,
/// including `Include` directives — resolved by ssh2-config relative
/// to `~/.ssh` or from absolute/`~` paths, with glob patterns).
pub fn resolve_alias_in(config_path: &Path, alias: &str) -> Result<ResolvedAlias, RemoteError> {
    let file = File::open(config_path).map_err(|e| {
        RemoteError::SshConfigAlias(format!(
            "cannot open ssh config at {} ({e}) — check that the file exists and readable",
            config_path.display()
        ))
    })?;
    let config = SshConfig::default()
        .parse(&mut BufReader::new(file), ParseRule::STRICT)
        .map_err(|e| {
            RemoteError::SshConfigAlias(format!(
                "failed to parse ssh config at {} ({e})",
                config_path.display()
            ))
        })?;

    // `query` returns default (all-None) params for an unmatched host,
    // so "unknown alias" is detected as "nothing was set at all". A
    // matched block with no HostName still counts as matched (host =
    // the alias itself, OpenSSH default).
    let params = config.query(alias);
    let matched = params.host_name.is_some()
        || params.user.is_some()
        || params.port.is_some()
        || params.identity_file.is_some()
        || params.proxy_jump.as_ref().is_some_and(|j| !j.is_empty());
    if !matched {
        return Err(RemoteError::SshConfigAlias(format!(
            "ssh config alias `{alias}` has no matching Host block in {} — \
             add a `Host {alias}` entry there (or fix the typo) and retry",
            config_path.display()
        )));
    }

    // v1 boundary: ProxyJump needs jump-host dialing the russh stack
    // does not implement. Fail explicitly instead of dialing the
    // target directly (which would bypass the jump and likely hang).
    if params.proxy_jump.as_ref().is_some_and(|j| !j.is_empty()) {
        return Err(RemoteError::SshConfigAlias(format!(
            "ssh config alias `{alias}` uses ProxyJump, not supported yet"
        )));
    }

    Ok(ResolvedAlias {
        host: params.host_name.unwrap_or_else(|| alias.to_string()),
        user: params.user,
        port: params.port,
        identity_file: params
            .identity_file
            .and_then(|files| files.into_iter().next()),
    })
}

/// One selectable alias for the settings UI's "Add SSH source" picker
/// (the alias dropdown this type feeds). The
/// `host`/`user` pair is a PREVIEW rendered as `user@host`; it is
/// derived at listing time and never persisted into the settings entry
/// (the entry keeps a live `sshConfig` reference instead).
/// `supported: false` marks a ProxyJump block — greyed out
/// in the picker with a "not supported yet" note (v1 boundary).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshAliasInfo {
    pub alias: String,
    /// HostName of the matching block, or the alias itself when the
    /// block sets none (OpenSSH default).
    pub host: String,
    /// `User` of the matching block, when present.
    pub user: Option<String>,
    /// False iff the block (or an earlier matching block) sets a
    /// non-empty ProxyJump — the russh stack cannot dial jump hosts.
    pub supported: bool,
}

/// The user's real `~/.ssh/config` path (actionable error when the
/// home directory cannot be determined). Consumed by the empty-state
/// guide in the add-source UI and by [`list_aliases`].
pub fn ssh_config_path() -> Result<PathBuf, RemoteError> {
    dirs::home_dir()
        .map(|home| home.join(".ssh").join("config"))
        .ok_or_else(|| {
            RemoteError::SshConfigAlias("cannot determine the user home directory".to_string())
        })
}

/// List every selectable Host alias from the user's real
/// `~/.ssh/config`. A missing or EMPTY config is NOT an error — the
/// empty Vec is exactly what drives the UI's empty-state guide
/// (path display + example config + manual fallback form). Only an
/// existing-but-unparseable file errors.
pub fn list_aliases() -> Result<Vec<SshAliasInfo>, RemoteError> {
    list_aliases_in(&ssh_config_path()?)
}

/// Test seam: same listing against an explicit config file path
/// (temp-file backed unit tests; `Include` directives are honored by
/// the same ssh2-config parse as [`resolve_alias_in`]).
///
/// Only LITERAL Host patterns become picker entries: wildcard
/// (`*`/`?`) patterns name host GROUPS rather than connectable
/// aliases (VS Code's picker shows them, but they have no stable
/// identity to store in settings), and negated clauses (`!host`) are
/// exclusions, never destinations. Each alias's preview comes from
/// `config.query(alias)` — the same first-match-wins merged
/// resolution the connect path uses, so the preview can never
/// disagree with what a later connect would dial. The parse uses
/// `ALLOW_UNKNOWN_FIELDS`: one unrecognized keyword in a hand-edited
/// file must not blank the whole picker.
pub fn list_aliases_in(config_path: &Path) -> Result<Vec<SshAliasInfo>, RemoteError> {
    if !config_path.exists() {
        // The documented "no config yet" state — empty list, not an
        // error (the UI shows the setup guide).
        return Ok(Vec::new());
    }
    let file = File::open(config_path).map_err(|e| {
        RemoteError::SshConfigAlias(format!(
            "cannot open ssh config at {} ({e}) — check that the file exists and readable",
            config_path.display()
        ))
    })?;
    let config = SshConfig::default()
        .parse(&mut BufReader::new(file), ParseRule::ALLOW_UNKNOWN_FIELDS)
        .map_err(|e| {
            RemoteError::SshConfigAlias(format!(
                "failed to parse ssh config at {} ({e})",
                config_path.display()
            ))
        })?;

    // Collect literal aliases in file order, deduped (a `Host a b`
    // clause and a later `Host a` must not produce two rows).
    let mut seen: HashSet<String> = HashSet::new();
    let mut aliases: Vec<String> = Vec::new();
    for host in config.get_hosts() {
        for clause in &host.pattern {
            if clause.negated {
                continue;
            }
            let pattern = clause.pattern.as_str();
            if pattern.is_empty() || pattern.contains('*') || pattern.contains('?') {
                continue;
            }
            if seen.insert(pattern.to_string()) {
                aliases.push(pattern.to_string());
            }
        }
    }

    Ok(aliases
        .into_iter()
        .map(|alias| {
            let params = config.query(&alias);
            SshAliasInfo {
                alias: alias.clone(),
                host: params.host_name.unwrap_or_else(|| alias.clone()),
                user: params.user,
                supported: !params
                    .proxy_jump
                    .as_ref()
                    .is_some_and(|j| !j.is_empty()),
            }
        })
        .collect())
}

/// Expand a leading `~` to the user's home directory. Applied
/// to EVERY key path before it reaches the key
/// loader — both IdentityFiles expanded out of ssh config and the
/// hand-typed `Key.keyPath` (fixing the v1 gap where a manual
/// `~/...` path was passed to the loader verbatim and failed).
///
/// `~` alone maps to the home directory; `~otheruser/…` is NOT
/// expanded (no user-database lookup here, unlike the ssh client) —
/// it passes through verbatim so the key loader surfaces a concrete
/// error instead of silently mis-resolving.
pub fn expand_tilde(path: &str) -> PathBuf {
    let Some(home) = dirs::home_dir() else {
        return PathBuf::from(path);
    };
    if path == "~" {
        return home;
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home.join(rest);
    }
    // Windows tolerance: a hand-edited file may carry "~\…" separators.
    if let Some(rest) = path.strip_prefix("~\\") {
        return home.join(rest);
    }
    PathBuf::from(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_config(dir: &Path, body: &str) -> std::path::PathBuf {
        let path = dir.join("config");
        std::fs::write(&path, body).expect("write config");
        path
    }

    #[test]
    fn known_alias_expands_host_user_port_identity() {
        let dir = tempdir().expect("tempdir");
        let path = write_config(
            dir.path(),
            "Host ali\n  HostName 192.0.2.10\n  User admin\n  Port 2222\n  IdentityFile ~/.ssh/id_ed25519\n",
        );
        let resolved = resolve_alias_in(&path, "ali").expect("resolve");
        assert_eq!(resolved.host, "192.0.2.10");
        assert_eq!(resolved.user.as_deref(), Some("admin"));
        assert_eq!(resolved.port, Some(2222));
        // ssh2-config already ~-expands IdentityFile paths; the raw
        // `~` prefix is gone (home-anchored instead).
        assert_eq!(
            resolved.identity_file.as_deref(),
            Some(
                dirs::home_dir()
                    .expect("home")
                    .join(".ssh")
                    .join("id_ed25519")
                    .as_path()
            )
        );
    }

    #[test]
    fn alias_without_hostname_defaults_to_alias_itself() {
        // OpenSSH semantics: `Host ali` + no HostName connects to "ali"
        // (e.g. a /etc/hosts name). Must count as matched.
        let dir = tempdir().expect("tempdir");
        let path = write_config(dir.path(), "Host ali\n  User admin\n");
        let resolved = resolve_alias_in(&path, "ali").expect("resolve");
        assert_eq!(resolved.host, "ali");
        assert_eq!(resolved.user.as_deref(), Some("admin"));
    }

    #[test]
    fn unknown_alias_is_actionable() {
        let dir = tempdir().expect("tempdir");
        let path = write_config(dir.path(), "Host other\n  HostName 192.0.2.99\n");
        let err = resolve_alias_in(&path, "ali").expect_err("no Host block matches");
        let msg = err.to_string();
        assert!(msg.contains("`ali`"), "names the alias: {msg}");
        assert!(
            msg.contains("Host ali"),
            "tells the user what to add: {msg}"
        );
    }

    #[test]
    fn missing_config_file_is_actionable() {
        let dir = tempdir().expect("tempdir");
        let err = resolve_alias_in(&dir.path().join("nope"), "ali").expect_err("missing file");
        assert!(err.to_string().contains("cannot open"), "{}", err);
    }

    #[test]
    fn proxy_jump_alias_fails_loudly() {
        let dir = tempdir().expect("tempdir");
        let path = write_config(
            dir.path(),
            "Host ali\n  HostName 192.0.2.10\n  ProxyJump bastion\n",
        );
        let err = resolve_alias_in(&path, "ali").expect_err("ProxyJump is a v1 boundary");
        assert_eq!(
            err,
            RemoteError::SshConfigAlias(
                "ssh config alias `ali` uses ProxyJump, not supported yet".to_string()
            ),
            "exact v1 boundary message"
        );
    }

    // ── alias listing (the alias dropdown picker) ────────────────────

    #[test]
    fn listing_reports_alias_host_user_previews() {
        let dir = tempdir().expect("tempdir");
        let path = write_config(
            dir.path(),
            "Host ali\n  HostName 192.0.2.10\n  User admin\n\
             Host bare\n  User op\n",
        );
        let list = list_aliases_in(&path).expect("list");
        assert_eq!(
            list,
            vec![
                SshAliasInfo {
                    alias: "ali".to_string(),
                    host: "192.0.2.10".to_string(),
                    user: Some("admin".to_string()),
                    supported: true,
                },
                // No HostName → the alias itself (OpenSSH default).
                SshAliasInfo {
                    alias: "bare".to_string(),
                    host: "bare".to_string(),
                    user: Some("op".to_string()),
                    supported: true,
                },
            ]
        );
    }

    #[test]
    fn listing_marks_proxy_jump_unsupported() {
        let dir = tempdir().expect("tempdir");
        let path = write_config(
            dir.path(),
            "Host plain\n  HostName 192.0.2.10\n\
             Host jumped\n  HostName 192.0.2.20\n  ProxyJump bastion\n",
        );
        let list = list_aliases_in(&path).expect("list");
        assert!(list[0].supported);
        assert!(!list[1].supported, "ProxyJump row is greyed out in the UI");
    }

    #[test]
    fn listing_skips_wildcards_negations_and_duplicates() {
        let dir = tempdir().expect("tempdir");
        let path = write_config(
            dir.path(),
            "Host *\n  User default\n\
             Host *.example.com\n\
             Host !excluded\n\
             Host pair1 pair2\n  HostName 192.0.2.1\n\
             Host pair1\n  HostName 192.0.2.9\n",
        );
        let list = list_aliases_in(&path).expect("list");
        let aliases: Vec<&str> = list.iter().map(|a| a.alias.as_str()).collect();
        assert_eq!(aliases, vec!["pair1", "pair2"], "literal, deduped, in order");
        // The preview uses merged query semantics: the later `Host
        // pair1` block does NOT override the earlier HostName
        // (first-match-wins), matching what connect would dial.
        assert_eq!(list[0].host, "192.0.2.1");
    }

    #[test]
    fn listing_missing_or_empty_config_is_an_empty_list_not_an_error() {
        let dir = tempdir().expect("tempdir");
        assert!(list_aliases_in(&dir.path().join("nope")).expect("missing file")
            .is_empty());
        let empty = write_config(dir.path(), "");
        assert!(list_aliases_in(&empty).expect("empty file").is_empty());
    }

    #[test]
    fn listing_tolerates_unknown_keywords() {
        // ALLOW_UNKNOWN_FIELDS: one unrecognized directive must not
        // blank the whole picker (hand-edited configs drift).
        let dir = tempdir().expect("tempdir");
        let path = write_config(
            dir.path(),
            "Host ali\n  HostName 192.0.2.10\n  SomeFutureKeyword yes\n",
        );
        let list = list_aliases_in(&path).expect("list");
        assert_eq!(list.len(), 1);
    }

    // ── ~ expansion ──────────────────────────────────────────────────

    #[test]
    fn tilde_home_form_expands() {
        let home = dirs::home_dir().expect("home");
        assert_eq!(expand_tilde("~"), home);
    }

    #[test]
    fn tilde_slash_form_expands() {
        let home = dirs::home_dir().expect("home");
        assert_eq!(expand_tilde("~/"), home);
        assert_eq!(
            expand_tilde("~/.ssh/id_ed25519"),
            home.join(".ssh").join("id_ed25519")
        );
    }

    #[test]
    #[cfg(windows)]
    fn tilde_backslash_form_expands() {
        // Hand-edited files on Windows may use backslash separators. On unix
        // a backslash is not a path separator, so this expectation — and the
        // `~\` tolerance in expand_tilde — only hold on Windows.
        let home = dirs::home_dir().expect("home");
        assert_eq!(
            expand_tilde("~\\.ssh\\id_ed25519"),
            home.join(".ssh").join("id_ed25519")
        );
    }

    #[test]
    fn absolute_and_relative_paths_pass_through() {
        assert_eq!(expand_tilde("C:/keys/k"), PathBuf::from("C:/keys/k"));
        assert_eq!(expand_tilde("/etc/ssh/k"), PathBuf::from("/etc/ssh/k"));
        assert_eq!(expand_tilde("relative/k"), PathBuf::from("relative/k"));
    }

    #[test]
    fn other_user_tilde_is_not_expanded() {
        // ~user has no resolver here; pass through so the key loader
        // surfaces the error instead of silently mis-resolving.
        assert_eq!(expand_tilde("~root/k"), PathBuf::from("~root/k"));
    }
}
