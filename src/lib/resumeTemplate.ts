/**
 * Resume-copy template (`sessions.resumeTemplate` setting).
 *
 * The session detail's Resume row renders the template and its copy button
 * copies the RENDERED string — what you see is what you copy. Placeholders:
 * - `{ResumeCommand}` — the provider-built resume command (e.g. `codex resume <id>`)
 * - `{WorkDir}` — the session's project directory
 * - `{SessionId}` — the session id
 *
 * Semantics (deliberately predictable, no magic):
 * - An EMPTY template (unset, cleared, or whitespace-only) falls back to the
 *   raw resume command — the shipped default keeps today's copy behavior.
 * - Newlines pass through: a multi-line template copies as multiple shell
 *   lines (each runs on paste — the PowerShell 5.1 answer, which has no &&).
 * - Unknown placeholder names (typos) render literally so the mistake is
 *   visible in the row instead of silently swallowed.
 * - No quoting/escaping is applied: the result is pasted into the user's own
 *   shell, and only the user knows which shell that is.
 */

export interface ResumeTemplateValues {
  resumeCommand: string | null | undefined;
  workDir: string | null | undefined;
  sessionId: string | null | undefined;
}

export function renderResumeTemplate(
  template: string | null | undefined,
  values: ResumeTemplateValues,
): string {
  const body = template?.trim() ? template : "{ResumeCommand}";
  return body.replace(/\{(\w+)\}/g, (match, name: string) => {
    switch (name) {
      case "ResumeCommand":
        return values.resumeCommand ?? "";
      case "WorkDir":
        return values.workDir ?? "";
      case "SessionId":
        return values.sessionId ?? "";
      default:
        return match;
    }
  });
}
