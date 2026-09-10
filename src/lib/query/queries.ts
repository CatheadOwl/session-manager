import { useQuery } from "@tanstack/react-query";
import {
  loadableSessionHandleOptionsFromMeta,
  sessionsApi,
  type AppMetadata,
  type ForkTreeResult,
} from "@/lib/api/sessions";
import { fetchProviders, fetchSettings, type SettingsSnapshot } from "@/lib/api/settings";
import type { SessionDetail, SessionMeta } from "@/types";
import { queryKeys } from "./keys";

export const useSessionsQuery = (scope: "active" | "archived" = "active") => {
  return useQuery<SessionMeta[]>({
    queryKey: queryKeys.sessions(scope),
    queryFn: async () => sessionsApi.list({ scope }),
    staleTime: 30 * 1000,
  });
};

export const useAppMetadataQuery = () => {
  return useQuery<AppMetadata>({
    queryKey: queryKeys.appMetadata(),
    queryFn: async () => sessionsApi.getAppMetadata(),
    staleTime: 30 * 1000,
  });
};

/**
 * Settings core (ADR 0006). Consumers read individual keys off
 * `data.values` (e.g. `update.autoCheck`). The cache is refreshed by the
 * app-level `settings-changed` listener, not per-consumer refetches.
 */
export const useSettingsQuery = () => {
  return useQuery<SettingsSnapshot>({
    queryKey: queryKeys.settings(),
    queryFn: async () => fetchSettings(),
    staleTime: 30 * 1000,
  });
};

/**
 * Provider id list for the settings source editor's provider picker.
 * Backed by the same registry as the `agents` CLI subcommand; effectively
 * static for a process lifetime.
 */
export const useProvidersQuery = () => {
  return useQuery<string[]>({
    queryKey: queryKeys.providers(),
    queryFn: async () => fetchProviders(),
    staleTime: Infinity,
  });
};

export const useSessionDetailQuery = (session?: SessionMeta | null) => {
  const handleOptions = loadableSessionHandleOptionsFromMeta(session);

  return useQuery<SessionDetail>({
    queryKey: queryKeys.sessionDetail(
      handleOptions?.providerId ?? "",
      handleOptions?.locator ?? handleOptions?.sourcePath,
      handleOptions?.sessionId,
    ),
    queryFn: async () => {
      if (!handleOptions) {
        throw new Error("Session detail query requires a loadable session");
      }

      return sessionsApi.getSessionDetail(handleOptions);
    },
    enabled: Boolean(handleOptions),
    staleTime: 30 * 1000,
  });
};

export const useForkTreeQuery = (
  scope: "active" | "archived" = "active",
  projectDir?: string,
  enabled = true,
) => {
  return useQuery<ForkTreeResult>({
    queryKey: queryKeys.forkTree(scope, projectDir),
    queryFn: async () => sessionsApi.computeForkTree({ scope, projectDir }),
    staleTime: 5 * 60 * 1000, // 5 minutes as per design doc
    // Fork-tree computation is the most expensive backend operation (full-file
    // reads + SHA256 hash chains). Only run it while the tree view is active —
    // flat-list users never pay for it. Switching to tree mode re-enables the
    // query and React Query fetches it on demand.
    enabled,
  });
};
