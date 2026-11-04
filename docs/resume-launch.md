---
description: Resume launch mode — gate, template placeholders, per-OS defaults, payload quoting rules and troubleshooting for sessions.resumeLaunchTemplate
---

# Resume launch

One click on a session's Resume row opens a shell in the session's project
directory and resumes the agent. Controlled by two settings in
`~/.session-manager/settings.json` (also editable in Settings → Sessions):

- `sessions.resumeTemplateEnabled` (bool, default `false`) — the mode switch.
  Off: the Resume row shows the raw resume command with a copy button (the
  shipped behavior). On: the row renders `sessions.resumeLaunchTemplate` and
  the button **executes** the rendered line. SSH remote sessions always keep
  raw + copy — their project directory is on the remote host and cannot be
  launched locally.
- `sessions.resumeLaunchTemplate` (string) — the command line to execute.

## How execution works

`{ResumeCommand}` renders to the provider-built resume command (e.g.
`codex resume <id>`); `{WorkDir}` is the project directory; `{SessionId}` is
the session id; unknown `{Names}` stay literal. The template is a SINGLE
command line — the editor is a one-line input, and if a hand-edited file
still carries newlines they are collapsed to spaces at spawn (a pasted
second line would otherwise execute as a second command). The app spawns the
rendered line through the platform shell — `cmd /S /D /C` on Windows, `sh -c`
elsewhere — with **the project directory set as the process working
directory**. On Windows that means the default template needs no `cd` at all;
on macOS/Linux the terminal launchers do not inherit the caller's working
directory, so their recipes keep a `cd` inside.

## Defaults (resolved per OS at runtime)

| OS | Default template |
|----|------------------|
| Windows | `pwsh -NoExit -Command "{ResumeCommand}"` when PowerShell 7+ is installed (probed via PATH); otherwise `powershell -NoExit -Command "{ResumeCommand}"` (Windows PowerShell 5.1, always present). The terminal is whatever the system provides — the shell is PowerShell either way, `-NoExit` is its keep-open |
| macOS | `osascript -e 'tell application "Terminal" to do script "cd "{WorkDir}" && {ResumeCommand}"'` (first use asks for Automation permission) |
| Linux | `x-terminal-emulator -e bash -c "cd "{WorkDir}" && {ResumeCommand}; exec bash"` (Debian alternatives pointer; `exec bash` keeps the terminal open) |

## Quoting rules (read before hand-editing)

- **The payload must reach your shell intact.** The whole line is parsed by
  the OUTER platform shell (cmd on Windows, sh elsewhere) before pwsh/bash/
  osascript see it — whatever sits between the payload and that outer shell
  must shield it.
- On Windows that means quotes around `{ResumeCommand}`: without them, any
  cmd-special character in the payload (`|`, `&`, …) is parsed by the outer
  shell and the command silently never runs (verified end-to-end; plain
  tokens like `codex resume <id>` happen to survive unquoted, which is why
  both forms can appear to work). This is the standard documented form
  (`about_PowerShell_exe`: from cmd, wrap the `-Command` value in double
  quotes).
- On macOS/Linux the DEFAULT recipes already embed the payload inside their
  own quoting (the `do script "…"` string, the `bash -c "…"` string) — do
  NOT add extra quotes around `{ResumeCommand}` there; it would corrupt the
  recipe.
- If your payload itself needs quotes, escape the inner ones for cmd (`\"`)
  — or avoid the problem with `-EncodedCommand`.

## Alternatives

```jsonc
{
  "sessions": {
    // PowerShell 7 via Windows Terminal:
    // "resumeLaunchTemplate": "wt -d \"{WorkDir}\" pwsh -NoExit -Command \"{ResumeCommand}\""
    // cmd instead of PowerShell — && in the payload must stay INSIDE the
    // quoted /k argument, or it binds to the outer shell:
    // "resumeLaunchTemplate": "cmd /k \"cd /d \"{WorkDir}\" && {ResumeCommand}\""
    // GNOME Terminal (keep-open via exec bash):
    // "resumeLaunchTemplate": "gnome-terminal -- bash -c \"cd '{WorkDir}' && {ResumeCommand}; exec bash\""
    // KDE: konsole -e bash -c "cd '{WorkDir}' && {ResumeCommand}; exec bash"
    // iTerm2 needs its own AppleScript; Terminal.app default covers macOS.
  }
}
```

## Troubleshooting

- **A console/terminal opens but the command is just echoed** — the payload
  was consumed as a string, not code. Restore the quotes around
  `{ResumeCommand}` (or reset the template to the default by emptying the
  field in Settings).
- **The shell opens in the wrong directory** — hand-edited template lost the
  `cd` (macOS/Linux) or wraps the payload in a launcher that changes
  directory; the default templates show the correct shape.
