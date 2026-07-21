/**
 * UI-only helpers for session display.
 *
 * Domain helpers (getSessionKey, getMetadataKey, deriveFolderList, FolderGroup)
 * live in lib/domain.ts — this file is for presentation transforms only.
 */

export function getRoleDisplay(role: string): { label: string; tone: string } {
  const normalized = role.toLowerCase();
  if (normalized === "assistant") return { label: "Assistant", tone: "assistant" };
  if (normalized === "user") return { label: "User", tone: "user" };
  if (normalized === "system") return { label: "System", tone: "system" };
  if (normalized === "tool") return { label: "Tool", tone: "tool" };
  return { label: role || "Unknown", tone: "unknown" };
}
