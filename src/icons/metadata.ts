/**
 * Provider icon metadata — display names, brand colors, categories.
 *
 * `defaultColor` is the brand's primary hex color (without `#`).
 * Set to `undefined` when the provider has no canonical brand color.
 */
export interface ProviderIconMeta {
  providerId: string;
  label: string;
  defaultColor?: string;
}

const metaMap: Record<string, ProviderIconMeta> = {
  claude:   { providerId: "claude",   label: "Claude",      defaultColor: "D97757" },
  gemini:   { providerId: "gemini",   label: "Gemini",      defaultColor: "4285F4" },
  codex:    { providerId: "codex",    label: "Codex",       defaultColor: "00A67E" },
  opencode: { providerId: "opencode", label: "OpenCode",    defaultColor: "211E1E" },
  openclaw: { providerId: "openclaw", label: "OpenClaw",    defaultColor: "FF4D4D" },
  hermes:   { providerId: "hermes",   label: "Hermes",      defaultColor: undefined },
  qoder:    { providerId: "qoder",    label: "Qoder",       defaultColor: "2ADB5C" },
};

export function getProviderMeta(providerId: string): ProviderIconMeta | undefined {
  return metaMap[providerId.toLowerCase()];
}
