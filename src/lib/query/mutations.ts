import { useMutation, useQueryClient } from "@tanstack/react-query";
import { sessionsApi, type DeleteSessionOptions, type AppMetadata } from "@/lib/api/sessions";
import { queryKeys } from "./keys";
import { useSingleSessionMutation, useBatchSessionMutation } from "./mutation-factory";

export const useDeleteSessionMutation = (sourceScope: "active" | "archived") =>
  useSingleSessionMutation(
    async (input: DeleteSessionOptions) => { await sessionsApi.delete(input); },
    sourceScope,
    [sourceScope],
  );

export const useArchiveSessionMutation = () =>
  useSingleSessionMutation(
    async (input: DeleteSessionOptions) => { await sessionsApi.archive(input); },
    "active",
    ["active", "archived"],
  );

export const useRestoreSessionMutation = () =>
  useSingleSessionMutation(
    async (input: DeleteSessionOptions) => { await sessionsApi.restore(input); },
    "archived",
    ["active", "archived"],
  );

export const useDeleteSessionsMutation = (sourceScope: "active" | "archived") =>
  useBatchSessionMutation(
    async (items: DeleteSessionOptions[]) => sessionsApi.deleteMany(items),
    sourceScope,
    [sourceScope],
  );

export const useArchiveSessionsMutation = () =>
  useBatchSessionMutation(
    async (items: DeleteSessionOptions[]) => sessionsApi.archiveMany(items),
    "active",
    ["active", "archived"],
  );

export const useRestoreSessionsMutation = () =>
  useBatchSessionMutation(
    async (items: DeleteSessionOptions[]) => sessionsApi.restoreMany(items),
    "archived",
    ["active", "archived"],
  );

export const useSetSessionStarredMutation = () => {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({ sessionKey, starred }: { sessionKey: string; starred: boolean }) => {
      await sessionsApi.setSessionStarred(sessionKey, starred);
      return { sessionKey, starred };
    },
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.appMetadata() });
    },
  });
};

export const useSetPinnedFoldersMutation = () => {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (folders: string[]) => {
      // setPinnedFolders returns the normalized list so the optimistic cache
      // write in onSuccess stays canonical even if a caller passes raw pins.
      return sessionsApi.setPinnedFolders(folders);
    },
    onSuccess: (folders) => {
      queryClient.setQueryData<AppMetadata>(queryKeys.appMetadata(), (current: AppMetadata | undefined) => ({
        sessions: current?.sessions ?? {},
        pinnedFolders: folders,
      }));
      queryClient.invalidateQueries({ queryKey: queryKeys.appMetadata() });
    },
  });
};
