import type { SessionMeta } from "@/types";
import { getSessionKey } from "./domain";

/**
 * Build an O(1) lookup map from sessions array keyed by getSessionKey().
 *
 * Multiple consumers (useSessionQueries, TreeView, batch delete) need
 * session-by-key lookups. Centralizing prevents redundant O(n) scans
 * and keeps the Map reference stable across renders.
 */
export function buildSessionMap(sessions: SessionMeta[]): Map<string, SessionMeta> {
  const map = new Map<string, SessionMeta>();
  for (const s of sessions) {
    map.set(getSessionKey(s), s);
  }
  return map;
}

/**
 * Look up a session from a pre-built map. Returns null for null/undefined key.
 */
export function getSessionFromMap(
  map: Map<string, SessionMeta>,
  key: string | null | undefined,
): SessionMeta | null {
  return key ? map.get(key) ?? null : null;
}
