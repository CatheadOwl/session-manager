/**
 * UI helpers for provider display.
 */
export function getProviderDisplay(providerId: string): {
  label: string;
  shortLabel: string;
} {
  switch (providerId) {
    case "claude":   return { label: "Claude Code", shortLabel: "Claude" };
    case "gemini":   return { label: "Gemini CLI", shortLabel: "Gemini" };
    case "openclaw": return { label: "OpenClaw",   shortLabel: "OpenClaw" };
    case "codex":    return { label: "Codex",       shortLabel: "Codex" };
    case "opencode": return { label: "OpenCode",    shortLabel: "OpenCode" };
    case "hermes":   return { label: "Hermes",      shortLabel: "Hermes" };
    case "qoder":    return { label: "Qoder",       shortLabel: "Qoder" };
    default:         return { label: providerId,    shortLabel: providerId };
  }
}
