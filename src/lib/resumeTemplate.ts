/**
 * Resume launch template (`sessions.resumeLaunchTemplate` setting).
 *
 * When launch mode is on, the session detail's Resume row renders the
 * template and its button EXECUTES the rendered line in the user's
 * terminal (Rust `run_shell_command`) instead of copying it. Placeholders:
 * - `{ResumeCommand}` — the provider-built resume command (e.g. `codex resume <id>`)
 * - `{WorkDir}` — the session's project directory
 * - `{SessionId}` — the session id
 *
 * Semantics (deliberately predictable, no magic):
 * - The template is a single command LINE authored by the user — terminal
 *   invocation included (e.g. `wt -d "{WorkDir}" cmd /k {ResumeCommand}`).
 *   The per-OS default comes from the Rust settings core (descriptor
 *   `default`) and is passed in by callers; blank means "use that default".
 * - Unknown placeholder names (typos) render literally so the mistake is
 *   visible in the row instead of silently swallowed.
 * - No quoting/escaping is applied: the line runs through the platform
 *   shell (`cmd` / `sh`), and only the user knows their terminal recipe.
 */

export interface ResumeTemplateValues {
  resumeCommand: string | null | undefined;
  workDir: string | null | undefined;
  sessionId: string | null | undefined;
}

/**
 * Deterministic example session (codex-flavored) backing the settings
 * preview and the chip tooltips.
 */
export const RESUME_TEMPLATE_EXAMPLE = {
  resumeCommand: "codex resume a1b2c3",
  workDir: "D:\\Document\\Projects\\demo",
  sessionId: "a1b2c3",
} satisfies ResumeTemplateValues;

/**
 * Effective-template resolution — the single place "blank means default"
 * lives. Gate OFF means no template at all (the Resume row keeps the raw
 * resume command + copy); gate ON with a blank template falls back to the
 * caller-supplied per-OS default (never stored in the settings file, so
 * the file stays sparse and hand-editing `""` means "use the default").
 */
export function effectiveResumeTemplate(
  enabled: boolean,
  template: string | null | undefined,
  defaultTemplate: string,
): string | null {
  if (!enabled) return null;
  return template?.trim() ? template : defaultTemplate;
}

export function renderResumeTemplate(
  template: string | null | undefined,
  values: ResumeTemplateValues,
): string {
  return template?.replace(/\{(\w+)\}/g, (match, name: string) => {
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
  }) ?? "";
}
