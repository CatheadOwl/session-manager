# Synthetic Claude Fixtures for Fork Tree Tests

This directory contains synthetic Claude session JSONL files used by the
`synthetic_claude_fork_tree` integration test in `fork_tree/mod.rs`.

## Purpose

These fixtures test **Path B** of the fork tree builder: the UUID chain
longest-common-prefix (LCP) heuristic. Unlike Codex sessions (Path A, which
uses explicit `forked_from_id`), Claude sessions carry message UUIDs that
persist across `--resume` operations. This gives the tree builder a strong
cross-session matching signal.

Key differences from Codex fixtures:
- No `forked_from_id` field anywhere -- Claude does not have it
- Each user event has a `uuid` field for LCP matching
- Subagent filtering is by filename prefix (`agent-*.jsonl`)

## Topology

```
A (root, 4 events) ──┬──→ B (fork_at=2, 4 events)   uuid=[A1,A2,B3,B4]
                     └──→ C (fork_at=3, 4 events)   uuid=[A1,A2,A3,C4]
D (root, 3 events) ──────→ E (fork_at=1, 3 events)   uuid=[D1,E2,E3]
F (root, 2 events) ────── standalone
G (root, 2 events) ────── standalone
H (root, 1 event)  ────── standalone (greeting "hello" filtered from hash_chain)
S1, S2                   ─ subagent (filtered out via agent- prefix)
```

**Note**: E has 3 events (same uuid_chain length as D) because Path B sorts
by UUID chain length ascending. A child with fewer events than its parent
would be sorted first and become the parent instead. This is a characteristic
of the LCP heuristic -- the child must have at least as many UUIDs as the
parent for correct identification.

## Regeneration

To regenerate these fixtures, run:

```powershell
cd codebase
python src-tauri/tests/scripts/gen-claude-fixtures.py
```

This script uses deterministic UUIDs (MD5-based) so the same fixture files
are produced every run.

## Format

Each `.jsonl` file contains:
1. A head line with `sessionId`, `cwd`, and `timestamp`
2. `"type":"user"` lines with `uuid`, `parentUuid`, and `message.content`
