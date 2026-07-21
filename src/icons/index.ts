/**
 * Provider icon registry.
 *
 * Two tiers:
 *   1. Inline SVG strings (via Vite ?raw) — for brand icons with specific colors
 *   2. null — provider has no dedicated icon; ProviderIcon falls back to initials
 *
 * Add a new provider by:
 *   1. Drop its .svg into src/icons/
 *   2. Add an import + entry below
 */
import claudeSvg from "./claude.svg?raw";
import geminiSvg from "./gemini.svg?raw";
import openaiSvg from "./openai.svg?raw";
import opencodeSvg from "./opencode.svg?raw";
import clawSvg from "./claw.svg?raw";
import qoderSvg from "./qoder.svg?raw";

/** Map of provider-id → inline SVG string */
const iconMap: Record<string, string> = {
  claude: claudeSvg,
  gemini: geminiSvg,
  codex: openaiSvg, // Codex uses OpenAI brand
  opencode: opencodeSvg,
  openclaw: clawSvg,
  qoder: qoderSvg,
  // hermes: no SVG — falls back to initials
};

/** Provider-ids we know about (used for enum checks) */
export const KNOWN_PROVIDERS = [
  "claude",
  "gemini",
  "codex",
  "opencode",
  "openclaw",
  "hermes",
  "qoder",
] as const;

export type ProviderId = (typeof KNOWN_PROVIDERS)[number];

/**
 * Return the raw SVG string for a provider, or null if none registered.
 */
export function getProviderSvg(providerId: string): string | null {
  return iconMap[providerId.toLowerCase()] ?? null;
}

/**
 * True if this provider has a dedicated SVG icon.
 */
export function hasProviderIcon(providerId: string): boolean {
  return providerId.toLowerCase() in iconMap;
}
