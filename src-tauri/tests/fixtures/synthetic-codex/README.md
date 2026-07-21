# Synthetic Codex Fixtures for Fork Tree Tests

This directory contains synthetic Codex session JSONL files used by the
`synthetic_codex_fork_tree` integration test in `fork_tree/mod.rs`.

## Purpose

These fixtures replace the original golden test data (private Codex sessions)
with public, reproducible test data. They exercise the same fork tree topologies
while containing zero private information.

## Topology

```
A (root, 4 events) ─────────→ B (fork_at=2, 5 events)
C (root, 4 events) ──┬──→ D (fork_at=2, 3 events)
                     └──→ E (fork_at=3, 4 events)
F (root, 2 events) ────────→ G (fork_at=1, 3 events)
H (root, 2 events) ────────→ I (fork_at=1, 2 events)
K, L, M, N(orphan) ──────── standalone roots
S1, S2                     ─ subagent (filtered out)
```

## Regeneration

To regenerate these fixtures, run:

```powershell
python tests/scripts/gen-codex-fixtures.py
```

Or use the `gen-fixtures` test helper that writes synthetic sessions to
a tempdir at runtime (see `build_tree_forked_from_id_*` tests in `mod.rs`).

## Format

Each `.jsonl` file contains:
1. A `session_meta` line with `id`, `cwd`, and optionally `forked_from_id`
2. `response_item` lines with `type: "message"`, `role: "user"` content
