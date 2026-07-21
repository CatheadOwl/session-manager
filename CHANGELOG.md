# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/),
and this project adheres to [Semantic Versioning](https://semver.org/).

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
