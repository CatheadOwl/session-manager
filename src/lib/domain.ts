import type { SessionMeta } from "@/types";
import { normalizeProjectDir } from "@/utils/format";

/**
 * Domain helpers for session identity, grouping, and metadata.
 *
 * These belong in lib/ (not components/) because hooks and lib code depend on them.
 */

/** Globally unique key for a session (includes sourcePath for file-level uniqueness). */
export const getSessionKey = (session: SessionMeta): string =>
  `${session.providerId}:${session.sessionId}:${session.sourcePath ?? ""}`;

/** Metadata key for star/pin state (providerId:sessionId, no sourcePath). */
export const getMetadataKey = (session: SessionMeta): string =>
  `${session.providerId}:${session.sessionId}`;

export interface FolderGroup {
  name: string;
  count: number;
}

/** Derive a sorted list of folder groups from a list of sessions. */
export const deriveFolderList = (sessions: SessionMeta[]): FolderGroup[] => {
  const map = new Map<string, number>();
  for (const session of sessions) {
    const folder = normalizeProjectDir(session.projectDir);
    map.set(folder, (map.get(folder) || 0) + 1);
  }
  return Array.from(map.entries())
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => a.name.localeCompare(b.name));
};
