<div align="center">

<img src="src-tauri/icons/128x128.png" alt="Session Manager" height="96" />

# Session Manager

**Browse, search, inspect, archive, and clean up local and remote (SSH) AI coding-agent sessions.**

[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue?style=flat-square)](https://github.com/CatheadOwl/session-manager/releases/latest)
[![Tauri](https://img.shields.io/badge/Tauri-v2-ffc131?style=flat-square&logo=tauri&logoColor=black)](https://tauri.app)
[![React](https://img.shields.io/badge/React-18-61dafb?style=flat-square&logo=react&logoColor=white)](https://react.dev)
[![Rust](https://img.shields.io/badge/Rust-1.85+-dea584?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)

[Overview](#overview) | [Install](#install) | [Features](#features) | [Screenshots](#screenshots) | [Providers](#supported-providers) | [Development](#development) | [Configuration](#configuration) | [Architecture](#architecture)

</div>

---

## Overview

Session Manager is a Tauri desktop app for working with AI coding-agent conversation logs. It scans known session directories — local ones by default, and remote (SSH) hosts registered as additional read-only sources — and gives you a three-column workspace for moving between folders, session lists, fork trees (branching views of sessions that share prompt history), and message detail.

The project was inspired by [CC Switch's Session Manager](https://github.com/farion1231/cc-switch/blob/main/docs/user-manual/en/3-extensions/3.4-sessions.md), with a narrower focus on session browsing and file-level management.

## Install

Download the latest installer from [GitHub Releases](https://github.com/CatheadOwl/session-manager/releases/latest):

| Platform | Artifacts |
|----------|-----------|
| Windows | `*-x64-setup.exe` (NSIS) or `*-x64_en-US.msi` |
| macOS | `*universal.dmg` — Apple Silicon and Intel |
| Linux | `.deb`, `.rpm`, or `.AppImage` |

- Updates are delivered by the built-in auto-updater (checks on startup; can be turned off in Settings).
- The Windows installer is not code-signed, so SmartScreen will show an "unknown publisher" warning — choose "More info → Run anyway" to proceed.
- [CHANGELOG.md](CHANGELOG.md) lists what changed in each release.

## Features

- **Project-folder navigation** - sessions grouped by working directory, with pinned folders and active/archived scopes.
- **List and fork-tree views** - flat list or a fork tree of sessions that share prompt history.
- **Session search** - filter the session list by title, summary, project path, or session id.
- **In-session search** - find text within the currently open session's messages, with match count and prev/next navigation.
- **Session detail** - full messages or compact Q&A pairs, with metadata, tool calls, and a Markdown rendering toggle.
- **Starred sessions** - mark important sessions and filter to them.
- **Archive and restore** - single or folder-level batch, between active and archived directories.
- **Batch delete** - select multiple sessions and send them to the system trash in one action.
- **Time-ranged Q&A export** - export the visible sessions as Q&A-distilled JSON or Markdown with full provenance; also scriptable headlessly (`session-manager export --help`).
- **SSH remote sources** - register remote hosts as read-only sources and browse their sessions alongside local ones ([details](docs/sources.md)).
- **Resume launch** - one click opens a terminal in the session's project directory and resumes the agent ([details](docs/resume-launch.md)).

## Screenshots

> [!NOTE]
> Screenshots are captured with synthetic demo data.

### Session list with project folders

![Session list overview](docs/assets/screenshots/overview-list.png)

### Fork tree view

![Fork tree view](docs/assets/screenshots/fork-tree.png)

### Q&A detail pane

![Q&A detail](docs/assets/screenshots/qa-detail.png)

### Archived sessions

![Archived scope](docs/assets/screenshots/archived.png)

## Supported Providers

| Provider | Status | Default session location |
|----------|--------|--------------------------|
| Claude Code | Stable | `~/.claude/projects/` |
| Codex | Stable | `~/.codex/sessions/` |
| Gemini CLI | Experimental | `~/.gemini/tmp/*/chats/` |
| OpenCode | Experimental | `$XDG_DATA_HOME/opencode/storage/` or `~/.local/share/opencode/storage/` |
| OpenClaw | Experimental | `~/.openclaw/agents/` |
| Hermes | Experimental | `~/.config/hermes/sessions/` |
| Qoder | Experimental | `~/.qoder/projects/`, `~/.qoder-cn/projects/` |
| Pi | Experimental | `~/.pi/agent/sessions/` |

> **Stable** = the author dogfoods these two daily, so they get first-class treatment.
>
> **Experimental** = adapters written for tools the author doesn't personally run — theoretically they work, practically… who knows? PRs & issue reports welcome.
>
> **Codex formats** - sessions are read in both the classic single-file rollout format and the paginated history format (Codex 0.153+).

## Development

### Prerequisites

- [Node.js](https://nodejs.org/) >= 20 and [pnpm](https://pnpm.io/) >= 10
- [Rust toolchain](https://rustup.rs/) >= 1.85
- [Tauri system prerequisites](https://tauri.app/start/prerequisites/) for your OS

### Run in Development

```bash
pnpm install
pnpm tauri dev
```

### Build a Desktop Bundle

```bash
pnpm tauri build
```

The installer is written under `src-tauri/target/release/bundle/`.

### Useful Checks

```bash
pnpm typecheck
pnpm build
cargo test --manifest-path src-tauri/Cargo.toml
```

## Configuration

Settings live in the app's Settings panel, backed by a hand-editable JSONC file (`~/.session-manager/settings.json`) — both edit the same data. The full key reference lives in **[docs/settings.md](docs/settings.md)**; per-feature details are linked from the Features list.

## Architecture

```text
src/                     # Frontend: React + TypeScript + Vite
├── components/sessions/ # Three-column session UI, list/tree/detail views
├── hooks/               # UI state, queries, mutations, search, interactions
├── lib/                 # Tauri API facade, domain helpers, query cache
├── icons/               # Provider brand SVGs and metadata
└── styles/              # Plain CSS, split by area (layout, sidebar, detail, tree…)

src-tauri/src/           # Backend: Rust + Tauri v2
├── commands/            # Tauri command handlers
├── session_manager/     # Scan, parse, read, metadata, archive, delete
│   ├── providers/       # Per-provider adapters
│   └── remote/          # SSH remote sources: russh connection, alias resolution, batch scan, transient cache
├── fork_tree/           # Hash-chain/UUID-chain fork detection and cache
├── cli.rs               # Dual-mode dispatch: CLI subcommands (export) run before the GUI launches
├── settings.rs          # Hand-editable settings core + IPC (sources, update checks)
├── config.rs            # Provider path discovery and env overrides
└── fs_utils.rs          # Filesystem traversal helpers
```

Key implementation points:

- **Hash-chain fork detection** - sessions are related by comparing user-input hash chains (SHA-256, first 8 hex); longest-common-prefix matching reconstructs lineage even when the provider records no explicit parent id.
- **Bounded metadata scan** - list view reads only the head/tail of each JSONL file instead of parsing full conversations.
- **Disposable fork-tree cache** - fork analysis persists a version-gated cache keyed by source path; missing entries are computed on demand, stale entries pruned, and losing the file triggers a clean recomputation.

## Tech Stack

| Layer | Technologies |
|-------|--------------|
| Frontend | React 18, TypeScript, Vite, TanStack Query, FlexSearch, lucide-react, react-markdown, remark-gfm |
| Backend | Rust, Tauri v2, serde, chrono, sha2, dirs, trash, russh, russh-sftp |
| Styling | Plain CSS (area-scoped) |
| Build | pnpm, cargo, Tauri CLI |

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for commit and workflow conventions; bug reports and feature requests go to the [issue tracker](https://github.com/CatheadOwl/session-manager/issues).

## License

[MIT](LICENSE)
