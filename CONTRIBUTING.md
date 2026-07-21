# Contributing to Session Manager

Thank you for your interest in contributing! This document covers the development workflow and conventions.

## Development Setup

### Prerequisites

- [Node.js](https://nodejs.org/) ≥ 20
- [pnpm](https://pnpm.io/) ≥ 10
- [Rust](https://www.rust-lang.org/tools/install) ≥ 1.85
- Platform-specific dependencies for [Tauri v2](https://tauri.app/start/prerequisites/)

### Install & Run

```bash
pnpm install
pnpm tauri dev
```

### Useful Commands

| Command | Purpose |
|---------|---------|
| `pnpm dev` | Start Vite dev server (frontend only) |
| `pnpm build` | Type-check + production build |
| `pnpm typecheck` | TypeScript check without emit |
| `pnpm tauri dev` | Full Tauri dev with hot reload |
| `cargo test --manifest-path src-tauri/Cargo.toml` | Run Rust tests |

## Project Structure

```
src/              # React frontend (TypeScript)
  components/     # UI components
  hooks/          # Custom React hooks
  lib/            # Tauri command bindings & shared logic
  styles/         # CSS modules
  utils/          # Utility functions
src-tauri/        # Rust backend (Tauri v2)
  src/commands/   # Tauri IPC command handlers
  src/session_manager/  # Provider adapters & session logic
  src/fork_tree/  # Fork tree computation
```

## Commit Convention

This project follows [Conventional Commits v1.0.0](https://www.conventionalcommits.org/).

```
<type>[scope]: <description>
```

### Types

| type | purpose | SemVer |
|------|---------|--------|
| feat | new feature | MINOR |
| fix | bug fix | PATCH |
| docs | documentation only | — |
| style | formatting (no logic change) | — |
| refactor | refactor (no feature/fix) | — |
| perf | performance improvement | PATCH |
| test | test-related | — |
| build | build system / dependencies | — |
| ci | CI configuration | — |
| chore | miscellaneous | — |
| revert | revert a commit | — |

### Rules

1. Use **imperative mood** (add / fix / update), no past tense, no trailing period
2. Subject ≤ 50 chars; add body after a blank line (≤ 72 chars per line) if needed
3. Scope is optional: `feat(tree): ...` or `feat: ...` (no empty parentheses)
4. Mark breaking changes with `!`: `feat(api)!: remove legacy endpoint`

### Common Scopes

`tree` `messages` `detail` `list` `provider` `fork` `ui` `ci`

### Examples

```
feat(messages): add Find-in-page search
fix(tree): resolve expand/collapse gap in virtualized mode
perf(list): virtualize flat session list
chore: bump version to 0.4.0
```

## Guidelines

- Keep changes focused and within scope.
- Do not add new dependencies without discussing first.
- If a markdown file exceeds 200 lines, split it into a folder with an `INDEX.md`.
- Run `pnpm typecheck` and `cargo test` before submitting a PR.

## License

By contributing, you agree that your contributions will be licensed under the project's [LICENSE](./LICENSE).

