import type { SessionMeta } from "@/types";
import { normalizeProjectDir } from "@/utils/format";

/**
 * Domain helpers for session identity, grouping, and metadata.
 *
 * These belong in lib/ (not components/) because hooks and lib code depend on them.
 */

const locatorKeyPart = (session: SessionMeta): string => {
  if (session.locator?.kind === "database") {
    return `database:${session.locator.path}:${session.locator.recordId ?? session.locator.record_id ?? session.sessionId}`;
  }

  if (session.locator?.kind === "file") {
    return `file:${session.locator.path}`;
  }

  return `file:${session.sourcePath ?? ""}`;
};

/** Globally unique key for a session (includes storage locator for data-level uniqueness). */
export const getSessionKey = (session: SessionMeta): string =>
  `${session.providerId}:${session.sessionId}:${locatorKeyPart(session)}`;

/** Metadata key for star/pin state (providerId:sessionId, no sourcePath). */
export const getMetadataKey = (session: SessionMeta): string =>
  `${session.providerId}:${session.sessionId}`;

export interface SessionLifecycleOperationOptions {
  providerId: string;
  sessionId: string;
  /**
   * Named for the existing mutation API. For file locators this is the file
   * locator path; for legacy sessions it falls back to SessionMeta.sourcePath.
   */
  sourcePath: string;
  locator?: SessionMeta["locator"];
}

export const getLifecycleOperationOptions = (
  session: SessionMeta,
): SessionLifecycleOperationOptions | undefined => {
  if (session.locator?.kind === "database") {
    return undefined;
  }

  const sourcePath = session.locator?.kind === "file"
    ? session.locator.path
    : session.sourcePath;
  if (!sourcePath) {
    return undefined;
  }

  return {
    providerId: session.providerId,
    sessionId: session.sessionId,
    sourcePath,
    locator: session.locator,
  };
};

export const supportsLifecycleOperations = (session: SessionMeta): boolean =>
  Boolean(getLifecycleOperationOptions(session));

export interface FolderGroup {
  name: string;
  count: number;
  /** Most recent session activity (lastActiveAt, falling back to createdAt) in this folder. */
  lastActiveAt: number;
}

/** Derive a sorted list of folder groups from a list of sessions. */
export const deriveFolderList = (sessions: SessionMeta[]): FolderGroup[] => {
  const map = new Map<string, { count: number; lastActiveAt: number }>();
  for (const session of sessions) {
    const folder = normalizeProjectDir(session.projectDir);
    const ts = session.lastActiveAt ?? session.createdAt ?? 0;
    const entry = map.get(folder);
    if (entry) {
      entry.count += 1;
      if (ts > entry.lastActiveAt) entry.lastActiveAt = ts;
    } else {
      map.set(folder, { count: 1, lastActiveAt: ts });
    }
  }
  return Array.from(map.entries())
    .map(([name, { count, lastActiveAt }]) => ({ name, count, lastActiveAt }))
    .sort((a, b) => a.name.localeCompare(b.name));
};

/**
 * Map pinned-folder keys onto canonical folder names.
 *
 * `AppMetadata.pinnedFolders` is a persisted store contract: every entry must
 * live in the same canonical space as folder names derived by
 * `deriveFolderList`, so `pinnedFolders.includes(folder.name)` and pin
 * write-backs stay consistent. Old builds stored pins with Windows backslashes
 * (e.g. `d:\...`); after separators were unified to `/` those no longer matched
 * `d:/...`. Normalizing at the metadata boundary — on read in `getAppMetadata`
 * and on write in `setPinnedFolders` — keeps the store canonical without a
 * one-time migration; dedupe guards against storage already holding both
 * spellings. Empty entries normalize to the `Unknown` sentinel (same as
 * `normalizeProjectDir` for missing project dirs); only reachable from
 * pre-corrupted storage, since pins are always written from canonical folder
 * names.
 */
export const normalizePinnedFolders = (folders: string[]): string[] => {
  const seen = new Set<string>();
  const result: string[] = [];
  for (const folder of folders) {
    const canonical = normalizeProjectDir(folder);
    if (!seen.has(canonical)) {
      seen.add(canonical);
      result.push(canonical);
    }
  }
  return result;
};
