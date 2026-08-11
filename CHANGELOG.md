# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- Add a Q&A table-of-contents popover for jumping between question-answer pairs in long sessions.

### Fixed

- Fix in-message search in Q&A mode so it works on rendered question/answer rows.
- Fix Claude/Qoder fork-tree detection to persist user-event UUID chains, preventing identical prompt templates from being grouped as forks.
- Refresh now also reloads the currently viewed session detail instead of leaving a stale pane.

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
