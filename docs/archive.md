---
description: Archive and restore semantics — per-provider directories, agent-side visibility, destination conflicts, retention interaction
---

# Archive and restore

Archive hides a session from its agent while keeping the file intact: the
session leaves the directory the agent scans and is kept in a sibling
archived directory, and restore moves it back. Archived sessions are listed
under the **Archived** scope in the left pane.

Entry points: the **Archive** / **Restore** buttons on a session's detail
page, and folder-level batch actions from the project sidebar.

How the move is performed differs by provider (see the table below). Unless
noted there, both directions are plain file moves — the session file itself
is never rewritten, and the move does not go through the system trash.

## Where sessions go

| Provider | Active directory | Archived directory | Mechanism |
|---|---|---|---|
| Claude Code | `~/.claude/projects` | `~/.claude/projects_archived` | File move |
| Codex | `~/.codex/sessions` | `~/.codex/archived_sessions` (Codex's own archive folder) | File move; Codex's own archive metadata is not updated (see below) |
| Gemini CLI | `~/.gemini/tmp` | `~/.gemini/tmp_archived` | File move |
| OpenClaw | `~/.openclaw/agents` | `~/.openclaw/agents_archived` | File move |
| OpenCode | `~/.local/share/opencode/storage` | `~/.local/share/opencode/storage_archived` | File move (V1 storage); 2.x SQLite: read-only |
| Hermes | `~/.config/hermes/sessions` | `~/.config/hermes/archived_sessions` | File move |
| Pi | `~/.pi/agent/sessions` | `~/.pi/agent/sessions_archived` | File move |

The per-project subdirectory layout inside the active directory is preserved
in the archive, so restoring puts every session back exactly where it came
from.

Sessions from [SSH remote sources](sources.md) and OpenCode 2.x SQLite
databases are read-only and cannot be archived.

## What the agent sees

The agent scans its active directory only, so an archived session disappears
from it: it drops out of the agent's resume picker, and resuming it by ID
fails until you restore it. This mirrors the behavior of Codex's own archive
(`codex archive`), which uses the same `archived_sessions` folder.

Two consequences worth knowing:

- **Archive is not deletion.** The file stays intact on disk and can always
  be restored from this app's Archived scope.
- **Archiving exempts a session from agent-side cleanup.** Claude Code
  deletes transcripts older than `cleanupPeriodDays` (30 days by default)
  from its active directory; a session moved to the archive is outside that
  scan and is kept indefinitely. Restoring puts it back under the cleanup
  window, so a long-restored session can still age out unless it is archived
  again or the setting is raised.

## Destination conflicts

If a session with the same file name already exists at the destination (for
example, restoring into an active directory where a file of that name is
present), the move is refused and a confirmation asks whether to replace the
existing session:

- **Cancel** keeps both sessions exactly as they are.
- **OK** replaces the existing destination session (including its sidecar
  directory) with the one being moved.

Folder-level batches behave the same way per session: sessions without a
conflict move normally, and the conflicting ones are offered as a group
("N sessions already exist… — replace them?"). Confirming replaces only that
subset; cancelling leaves them archived.

## Independence from other archive states

Session Manager's archive state is its own. In particular:

- The Claude Code VS Code extension keeps its own archived-sessions list
  (inside VS Code), which this app does not read or write. Archiving here
  does not archive in the extension and vice versa; the extension only hides
  sessions from its own list, while archiving here moves the file.
- Codex archive here is currently file-level: sessions move into
  `archived_sessions`, which Codex treats as archived by path. Codex's own
  session database is not updated — it still points at the original path, so
  while a session is archived here it cannot be resumed by ID or unarchived
  by Codex itself until you restore it here.

## Troubleshooting

- **A session is missing from `claude --resume` / the agent's picker** — check
  whether it is archived here (Archived scope in the left pane) and restore it.
- **"Destination conflict" on every move** — a same-named session already
  exists at the destination. Cancel keeps both; inspect the two sessions
  (each shows its full path in Metadata) before deciding to replace.
- **An archived Codex session cannot be resumed by ID** — restore it here
  first; Codex's session database still points at the original path while
  the file is archived.
