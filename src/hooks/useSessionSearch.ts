import { useCallback, useMemo, useRef } from "react";
import FlexSearch from "flexsearch";
import type { SessionMeta } from "@/types";

interface UseSessionSearchOptions {
  sessions: SessionMeta[];
  providerFilter: string;
}

interface UseSessionSearchResult {
  search: (query: string) => SessionMeta[];
}

export function useSessionSearch({
  sessions,
  providerFilter,
}: UseSessionSearchOptions): UseSessionSearchResult {
  const filteredByProvider = useMemo(() => {
    if (providerFilter === "all") return sessions;
    return sessions.filter((session) => session.providerId === providerFilter);
  }, [sessions, providerFilter]);

  /**
   * Lazily-built FlexSearch index, cached per `filteredByProvider` scope.
   *
   * Building the index is O(n) in session count and is pure waste while the
   * search box is empty — the empty-query path below never touches it. So defer
   * construction until the first non-empty query, and rebuild only when the
   * scoped session list actually changes (new array identity). Keeping the build
   * out of the eager render path removes the per-folder-click rebuild that made
   * selecting a folder with many sessions laggy.
   */
  const indexRef = useRef<{
    scope: SessionMeta[];
    index: InstanceType<typeof FlexSearch.Index>;
  } | null>(null);

  const search = useCallback(
    (query: string): SessionMeta[] => {
      const needle = query.trim();

      if (!needle) {
        return [...filteredByProvider].sort((a, b) => {
          const aTs = a.lastActiveAt ?? a.createdAt ?? 0;
          const bTs = b.lastActiveAt ?? b.createdAt ?? 0;
          return bTs - aTs;
        });
      }

      // Build (or rebuild) the index once per session list scope.
      if (!indexRef.current || indexRef.current.scope !== filteredByProvider) {
        const nextIndex = new FlexSearch.Index({
          tokenize: "full",
          resolution: 9,
        });

        filteredByProvider.forEach((session, idx) => {
          const metaContent = [
            session.sessionId,
            session.title,
            session.summary,
            session.projectDir,
            session.sourcePath,
          ]
            .filter(Boolean)
            .join(" ");

          nextIndex.add(idx, metaContent);
        });

        indexRef.current = { scope: filteredByProvider, index: nextIndex };
      }

      const results = indexRef.current.index.search(needle, {
        limit: filteredByProvider.length,
      }) as number[];

      return results.map((idx) => filteredByProvider[idx]);
    },
    [filteredByProvider],
  );

  return { search };
}
