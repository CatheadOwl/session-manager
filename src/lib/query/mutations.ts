import { useMutation, useQueryClient } from "@tanstack/react-query";
import { sessionsApi, type DeleteSessionOptions, type AppMetadata } from "@/lib/api/sessions";
import { queryKeys } from "./keys";
import { useSingleSessionMutation, useBatchSessionMutation } from "./mutation-factory";

export const useDeleteSessionMutation = () =>
  useSingleSessionMutation(
    async (input: DeleteSessionOptions) => { await sessionsApi.delete(input); },
    "active",
    ["active"],
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

export const useDeleteSessionsMutation = () =>
  useBatchSessionMutation(
    async (items: DeleteSessionOptions[]) => sessionsApi.deleteMany(items),
    "active",
    ["active"],
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
      await sessionsApi.setPinnedFolders(folders);
      return folders;
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
