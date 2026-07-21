import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { DeleteSessionOptions, DeleteSessionResult } from "@/lib/api/sessions";
import type { SessionMeta } from "@/types";
import { queryKeys } from "./keys";

const sameSession = (session: SessionMeta, input: DeleteSessionOptions) =>
  session.providerId === input.providerId &&
  session.sessionId === input.sessionId &&
  session.sourcePath === input.sourcePath;

const removeSuccessfulSessionsFromCache = (
  queryClient: ReturnType<typeof useQueryClient>,
  scope: "active" | "archived",
  outcomes: DeleteSessionResult[],
) => {
  const successful = outcomes.filter((outcome) => outcome.success);
  queryClient.setQueryData<SessionMeta[]>(queryKeys.sessions(scope), (current: SessionMeta[] | undefined) =>
    (current ?? []).filter((session: SessionMeta) => !successful.some((outcome) => sameSession(session, outcome))),
  );
  for (const outcome of successful) {
    queryClient.removeQueries({ queryKey: queryKeys.sessionDetail(outcome.providerId, outcome.sourcePath) });
  }
};

const removeSingleSessionFromCache = (
  queryClient: ReturnType<typeof useQueryClient>,
  scope: "active" | "archived",
  input: DeleteSessionOptions,
) => {
  queryClient.setQueryData<SessionMeta[]>(queryKeys.sessions(scope), (current: SessionMeta[] | undefined) =>
    (current ?? []).filter((session: SessionMeta) => !sameSession(session, input)),
  );
  queryClient.removeQueries({ queryKey: queryKeys.sessionDetail(input.providerId, input.sourcePath) });
};

/**
 * Create a mutation that operates on a single session.
 * Removes the session from the source scope cache and invalidates both active/archived queries.
 */
export function useSingleSessionMutation(
  apiMethod: (input: DeleteSessionOptions) => Promise<void>,
  sourceScope: "active" | "archived",
  invalidateTargets: ("active" | "archived")[],
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (input: DeleteSessionOptions) => {
      await apiMethod(input);
      return input;
    },
    onSuccess: async (input) => {
      removeSingleSessionFromCache(queryClient, sourceScope, input);
      for (const scope of invalidateTargets) {
        await queryClient.invalidateQueries({ queryKey: queryKeys.sessions(scope) });
      }
    },
  });
}

/**
 * Create a mutation that operates on multiple sessions.
 * Removes successful sessions from the source scope cache and invalidates both active/archived queries.
 */
export function useBatchSessionMutation(
  apiMethod: (items: DeleteSessionOptions[]) => Promise<DeleteSessionResult[]>,
  sourceScope: "active" | "archived",
  invalidateTargets: ("active" | "archived")[],
) {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (items: DeleteSessionOptions[]) => apiMethod(items),
    onSuccess: async (outcomes) => {
      removeSuccessfulSessionsFromCache(queryClient, sourceScope, outcomes);
      for (const scope of invalidateTargets) {
        await queryClient.invalidateQueries({ queryKey: queryKeys.sessions(scope) });
      }
    },
  });
}
