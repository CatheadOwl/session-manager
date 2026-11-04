---
description: settings.json reference — file location, JSONC/sparse rules, full key table (version, update.autoCheck, sources[], sessions.*) and loading behavior
---

# Settings reference

Settings are editable from the app's Settings panel, and also live in a hand-editable file (VS Code-style) — both edit the same data:

```
~/.session-manager/settings.json
```

The file is JSONC (comments allowed) and stores only overridden keys: values equal to the built-in defaults are omitted, and `version` is always written. Restart the app after hand-editing.

## Key reference

| Key | Type | Default | Semantics |
|-----|------|---------|-----------|
| `version` | number | *(always written)* | Migration anchor; current = `1` |
| `update.autoCheck` | bool | `true` | `false` skips the automatic update check on startup (manual retry still works) |
| `sources[]` | array | `[]` | Extra session sources, scanned in addition to the built-in provider locations. Entries carry a `kind` tag: local entries have no `kind` key, SSH entries carry `"kind": "ssh"`. Entry semantics: **[sources.md](sources.md)** |
| `sessions.resumeTemplateEnabled` | bool | `false` | Launch-mode gate for the session detail's Resume row (off: raw resume command + copy button; on: the rendered template's launch button executes it). SSH remote sessions always keep raw + copy — their project path is remote. Full semantics: **[resume-launch.md](resume-launch.md)** |
| `sessions.resumeLaunchTemplate` | string | *(per-OS default)* | The single command line the launch button executes when the gate is on; an empty string means the per-OS default. Placeholders, per-OS defaults and quoting rules: **[resume-launch.md](resume-launch.md)** |

## Loading behavior

Loading is lenient — a broken file never blocks startup (defaults are used), unknown keys are preserved on programmatic saves, and wrong-typed known keys fall back to defaults with a logged warning.

> [!NOTE]
> Comments are a hand-editing convenience only: the first time the app saves the file programmatically (e.g. changing a setting from the UI), comments are dropped.
