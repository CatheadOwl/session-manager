---
description: Extra session sources — local home mirrors and SSH remotes, covering GUI setup, settings-file entries, provider discovery, caching and read-only behavior
---

# Sources: local mirrors & SSH remotes

Extra sources are scanned in addition to the built-in provider locations, so you can browse sessions that live elsewhere — another home directory on this machine, or another machine entirely (for example a Linux dev server where Codex or Claude Code runs over VS Code Remote SSH).

## GUI setup (SSH)

Open **Settings** (the gear at the bottom of the folder strip) → **Sources** → **Add SSH source**. Two entry paths, both with a connection test before saving:

- **Alias picker** - lists `Host` aliases from your `~/.ssh/config`; connection settings and keys are read live from the config block (ssh-agent identities first, then the block's `IdentityFile`). `ProxyJump` hosts are not supported yet and are greyed out.
- **Manual form** ("Advanced") - enter host / port / user yourself and pick an auth mode: ssh-agent, or an explicit key file (`~` expanded).

## Behavior

- Provider session directories are discovered automatically under the remote home (`~/.codex/sessions/`, `~/.claude/projects/`, …) — same providers as local, no remote path configuration. A **local source** works the same way: its `path` points at an alternate home root and every provider's standard directory is discovered under it.
- Remote sessions appear in the same list and detail views; message content is fetched over SSH when you open a session and cached locally for re-opens and exports.
- Remote sources are **currently read-only**: delete / archive / restore remain local-only.
- The remote cache lives in the OS cache directory (`%LOCALAPPDATA%\session-manager\remote-cache` on Windows) and is safe to delete at any time — deleted entries are simply refetched on the next open.

## Settings-file entries

```jsonc
{
  "version": 1,
  "update": { "autoCheck": false },
  "sources": [
    // local: alternate home root, providers discovered under it
    { "path": "D:\\home-mirror", "enabled": true },
    // ssh: alias mode — settings read live from ~/.ssh/config
    { "kind": "ssh", "id": "devbox", "host": "", "user": "", "port": 22,
      "auth": { "mode": "sshConfig", "alias": "dev" } },
    // ssh: agent / explicit key modes
    { "kind": "ssh", "id": "build", "host": "192.0.2.10", "user": "admin", "port": 2222,
      "auth": { "mode": "key", "keyPath": "~/.ssh/id_ed25519" } }
  ]
}
```

**Local source** — `path` points at an **alternate home root** (above). There is no per-entry `provider` field anymore; a legacy `provider` key from an older file is preserved but ignored.

**SSH source** — `host`/`port`/`user` plus an `auth` block with three modes — `sshConfig` (alias from `~/.ssh/config`), `agent` (ssh-agent), or `key` (explicit key file).

Common entry fields: `enabled` (default `true`; disabled entries are kept but not scanned), `id` (required for SSH, optional for local), `label` (optional, SSH).
