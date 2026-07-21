import { useMemo } from "react";
import { useSessionSearch } from "@/hooks/useSessionSearch";
import {
  useAppMetadataQuery,
  useForkTreeQuery,
  useSessionDetailQuery,
  useSessionsQuery,
} from "@/lib/query/queries";
import { buildSessionMap, getSessionFromMap } from "@/lib/session-map";
import type { SessionMeta } from "@/types";
import { normalizeProjectDir } from "@/utils/format";
import { deriveFolderList, getMetadataKey, getSessionKey } from "@/lib/domain";

/**
 * Queries + derived data for the session manager.
 * No UI state, no mutations — strictly data fetching and computation.
 */
export function useSessionQueries(
  scope: "active" | "archived",
  selectedFolder: string,
  selectedKey: string | null,
  search: string,
) {
  // ─── Raw queries ──────────────────────────────────────────────────
  const sessionsQuery = useSessionsQuery(scope);
  const sessions: SessionMeta[] = sessionsQuery.data ?? [];
  const appMetadataQuery = useAppMetadataQuery();
  const pinnedFolders: string[] = appMetadataQuery.data?.pinnedFolders ?? [];

  // Fork tree
  const folderForTree = selectedFolder !== "all" && selectedFolder !== "Unknown"
    ? selectedFolder
    : undefined;
  const forkTreeQuery = useForkTreeQuery(scope, folderForTree);
  const treeRoots = forkTreeQuery.data?.roots ?? [];
  const treeTotalSessions = forkTreeQuery.data?.totalSessions ?? 0;

  // ─── Derived data ─────────────────────────────────────────────────
  const starredMap = useMemo(() => {
    const map = new Map<string, boolean>();
    if (appMetadataQuery.data) {
      for (const [key, meta] of Object.entries(appMetadataQuery.data.sessions)) {
        if ((meta as { starred: boolean }).starred) map.set(key, true);
      }
    }
    return map;
  }, [appMetadataQuery.data]);

  /** O(1) session-by-key lookup, shared with TreeView and batch delete. */
  const sessionMap = useMemo(() => buildSessionMap(sessions), [sessions]);

  const folderGroups = useMemo(() => deriveFolderList(sessions), [sessions]);

  const folderFiltered = useMemo(
    () =>
      selectedFolder === "all"
        ? sessions
        : sessions.filter((s) => normalizeProjectDir(s.projectDir) === selectedFolder),
    [sessions, selectedFolder],
  );

  const { search: searchSessions } = useSessionSearch({
    sessions: folderFiltered,
    providerFilter: "all",
  });

  const filteredSessions = useMemo(
    () => searchSessions(search),
    [searchSessions, search],
  );

  // Selected session (O(1) via sessionMap)
  const selectedSession = useMemo(
    () => getSessionFromMap(sessionMap, selectedKey),
    [sessionMap, selectedKey],
  );

  const selectedMetaKey = selectedSession ? getMetadataKey(selectedSession) : null;
  const isStarred = selectedMetaKey ? starredMap.has(selectedMetaKey) : false;

  // Session detail query (only fires when a session is selected)
  const sessionDetailQuery = useSessionDetailQuery(
    selectedSession?.providerId,
    selectedSession?.sourcePath,
  );

  return {
    sessionsQuery,
    sessions,
    sessionMap,
    appMetadataQuery,
    pinnedFolders,
    forkTreeQuery,
    treeRoots,
    treeTotalSessions,
    starredMap,
    folderGroups,
    filteredSessions,
    selectedSession,
    isStarred,
    sessionDetailQuery,
  };
}
