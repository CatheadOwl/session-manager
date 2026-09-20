# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.4.0] - 2026-09-20

### Added

- Add support for Codex 0.153+ sessions (paginated history format): new-format sessions now load their complete message stream — user questions, assistant answers, and tool cards.

### Fixed

- Fix Codex system blocks (app context, skills instructions, collaboration mode, multi-agent role hints) rendering as plain message text; they now show as collapsible system cards.
- Fix Codex message boundaries when a system block's closing marker is glued to adjacent content, which could split or swallow surrounding messages.

## [0.3.0] - 2026-09-11

### Added

- Add SSH remote sources: register a remote host as a session source in Settings (ssh-config alias picker, connection test, or manual entry) and browse its sessions read-only; provider roots on the remote host are discovered automatically, and remote sessions cannot be deleted, archived, or restored.
- Add a Settings panel to manage session sources — local sources point at mirrored home directories and SSH sources at remote hosts — with a shared enable toggle, remove, and a scan overlay showing what each source resolves to.
- Add Q&A session export: export the visible sessions (folder, search, star filter, time range, or All time with a large-export confirmation) as Q&A-distilled output with full provenance, tool placeholders and system blocks excluded; in selection mode, export covers the checked sessions instead. Results report in a floating toast, and overwriting an existing file is an explicit opt-in.
- Q&A export also covers SSH remote sessions: their content is fetched over SSH on first export (~0.2s per session, cached afterwards — re-exports are free) and the export provenance records the remote source and path. A remote session that fails to fetch is skipped individually; the rest of the export continues.
- Add a command-line mode: time-ranged Q&A export runs headlessly from a terminal (JSONL output, short flags), and `--help`/`--version` answer on the console instead of launching the GUI.
- Add a tri-state select-all control (all / partial / none) to the session list selection mode.

### Changed

- Session search now also matches session-id fragments.

### Fixed

- Fix batch delete acting on checked sessions that are no longer visible: switching folder, search, time range, star filter, or scope now drops out-of-view checks, so selection and batch delete always operate on the visible list.
- Fix invalid nested buttons inside session rows — invalid HTML that broke assistive-tech interaction.

## [0.2.4] - 2026-08-13

### Added

- Add a Q&A table-of-contents popover for jumping between question-answer pairs in long sessions.

### Fixed

- Fix in-message search in Q&A mode so it works on rendered question/answer rows.
- Fix Claude/Qoder fork-tree detection to persist user-event UUID chains, preventing identical prompt templates from being grouped as forks.
- Refresh now also reloads the currently viewed session detail instead of leaving a stale pane.
- Fix deleting a session in the archived view not removing it from the list until a manual refresh.

## [0.2.3] - 2026-08-09

### Changed

- Cap the OpenCode session list to the newest 1000 sessions to keep loading fast.

### Fixed

- Prevent session discovery from hanging when a session folder contains a symlink or junction loop.
- Fix row-height glitches in the session list when search or the star filter reorders items.
- Fix Claude and OpenCode sessions showing a blank preview in the session list.

## [0.2.2] - 2026-08-03

### Fixed

- Fix pinned folders silently unpinning when the stored project-directory separator spelling (`d:\proj`) no longer matched the canonical form (`d:/proj`).

## [0.2.1] - 2026-08-02

### Added

- Add sort/filter controls to the left-side folder panel. ([#1](https://github.com/CatheadOwl/session-manager/issues/1))
- Add one-click COPY buttons to session metadata (resume command, etc.). ([#2](https://github.com/CatheadOwl/session-manager/issues/2))

### Fixed

- Fix fork-tree nodes rendering as "Not in current scope" due to a mismatched session key format.

## [0.2.0] - 2026-08-01

### Added

- Add read-only OpenCode SQLite session discovery and detail loading.
- Add a Pi (Earendil) session provider.

### Changed

- Use structured session locators to support both file-backed and database-backed sessions.
- Treat OpenCode database-backed sessions as read-only for delete, archive, restore, and fork tree in this release.
- Show OpenCode tool results mapped from session state, with a fallback for legacy tool-result-only parts.

## [0.1.1] - 2026-07-26

### Changed

- macOS now ships as a Universal binary for Apple Silicon and Intel.
- Use the stable `com.catheadowl.session-manager` application identifier.
- Improve automatic update reliability across all platforms.

## [0.1.0] - 2026-07-21

### Added

- Three-column workspace: project folders, session list / fork tree, message detail.
- Fork-tree view with hash-chain and UUID-chain divergence detection.
- Local full-text search (FlexSearch) across title, summary, path, provider, and session id.
- In-message search with match count, prev/next navigation, and inline highlighting.
- Q&A pair and full-message detail modes with Markdown rendering toggle.
- Starred sessions and pinned folders.
- Archive / restore (single and folder-level batch).
- Batch delete with provider-root safety validation.
- Multi-provider adapters: Claude Code, Codex, Gemini CLI, OpenCode, OpenClaw, Hermes, Qoder.
- Window state persistence (size, position, maximized).
- Tauri Updater integration for automatic updates.
