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

/**
 * Recommended template — the natural fallback for a blank template while
 * the gate is on (see effectiveResumeTemplate), and the editor's gray
 * placeholder so the default is visible before anything is typed.
 */
export const DEFAULT_RESUME_TEMPLATE = 'cd "{WorkDir}"\n{ResumeCommand}';

/**
 * Effective-template resolution — the single place "empty means default"
 * lives. Gate OFF means no template at all (renderResumeTemplate falls back
 * to the raw resume command); gate ON with a blank template falls back to
 * the recommended default. The default is a RENDER-time fallback mirroring
 * the editor's placeholder — it is never written to the settings file, so
 * the file stays sparse and hand-editing `""` means "use the default".
 */
export function effectiveResumeTemplate(
  enabled: boolean,
  template: string | null | undefined,
): string | null {
  if (!enabled) return null;
  return template?.trim() ? template : DEFAULT_RESUME_TEMPLATE;
}

/**
 * Deterministic example session (codex-flavored) backing the settings
 * preview and the chip tooltips. `satisfies` keeps the non-null field types
 * (consumers present them as plain strings).
 */
export const RESUME_TEMPLATE_EXAMPLE = {
  resumeCommand: "codex resume a1b2c3",
  workDir: "D:\\Document\\Projects\\demo",
  sessionId: "a1b2c3",
} satisfies ResumeTemplateValues;

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
