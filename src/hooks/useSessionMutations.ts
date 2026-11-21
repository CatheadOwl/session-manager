import { useCallback } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
  invocationErrorMessage,
  isDestinationConflict,
  type DeleteSessionOptions,
  type DeleteSessionResult,
} from "@/lib/api/sessions";
import {
  useArchiveSessionMutation,
  useArchiveSessionsMutation,
  useDeleteSessionMutation,
  useDeleteSessionsMutation,
  useRestoreSessionMutation,
  useRestoreSessionsMutation,
  useSetPinnedFoldersMutation,
  useSetSessionStarredMutation,
} from "@/lib/query/mutations";
import type { SessionMeta } from "@/types";
import { normalizeProjectDir } from "@/utils/format";
import { getLifecycleOperationOptions, getMetadataKey } from "@/lib/domain";

export type FolderLifecycleAction = "archive" | "restore";

interface UseSessionMutationsOptions {
  onSessionDeleted: () => void;
  onSessionArchived: () => void;
  onSessionRestored: () => void;
  onFolderOperationComplete: (outcomes: DeleteSessionResult[], action: FolderLifecycleAction) => void;
}

/**
 * Mutation handlers for the session manager.
 * Receives data dependencies (sessions, pinnedFolders, starredMap) and callbacks for UI state updates,
 * keeping the hook free of direct UI state management.
 */
export function useSessionMutations(
  scope: "active" | "archived",
  sessions: SessionMeta[],
  pinnedFolders: string[],
  starredMap: Map<string, boolean>,
  options: UseSessionMutationsOptions,
) {
  const { onSessionDeleted, onSessionArchived, onSessionRestored, onFolderOperationComplete } = options;

  // ─── Raw mutations ────────────────────────────────────────────────
  const deleteMutation = useDeleteSessionMutation(scope);
  const deleteSessionsMutation = useDeleteSessionsMutation(scope);
  const archiveMutation = useArchiveSessionMutation();
  const restoreMutation = useRestoreSessionMutation();
  const archiveSessionsMutation = useArchiveSessionsMutation();
  const restoreSessionsMutation = useRestoreSessionsMutation();
  const setPinnedFoldersMutation = useSetPinnedFoldersMutation();
  const setStarredMutation = useSetSessionStarredMutation();

  // ─── Data helpers ─────────────────────────────────────────────────
  const getFolderOperationItems = useCallback(
    (folder: string, sourceSessions: SessionMeta[] = sessions): { items: DeleteSessionOptions[]; skippedCount: number } => {
      if (folder === "all" || folder === "Unknown") return { items: [], skippedCount: 0 };
      const folderSessions = sourceSessions
        .filter((session) => normalizeProjectDir(session.projectDir) === folder)
      const items = folderSessions
        .map(getLifecycleOperationOptions)
        .filter((item): item is DeleteSessionOptions => Boolean(item));
      return { items, skippedCount: folderSessions.length - items.length };
    },
    [sessions],
  );

  // ─── Single-session handlers ──────────────────────────────────────
  const confirmDeleteSession = useCallback(
    (session: SessionMeta) => {
      const input = getLifecycleOperationOptions(session);
      if (!input) return;

      deleteMutation.mutate(
        input,
        {
          onSuccess: () => onSessionDeleted(),
          // Tauri invoke rejects with raw strings, not Error instances.
          onError: (error) => window.alert(invocationErrorMessage(error)),
        },
      );
    },
    [deleteMutation, onSessionDeleted],
  );

  const executeBatchDelete = useCallback(
    (sessionsToDelete: DeleteSessionOptions[]) => {
      if (sessionsToDelete.length === 0) return;

      // The batch command RESOLVES with per-item outcomes — per-item failures
      // never reject, so without inspecting the data a batch where every item
      // failed would report success. Notify only if something actually was
      // deleted (the mutation layer removes only successful outcomes from the
      // cache, so a zero-success batch leaves the lists untouched), and
      // surface the failed items' error strings through the window.alert
      // fall-through, mirroring the page's capped folder-op alert shape.
      deleteSessionsMutation.mutate(sessionsToDelete, {
        onSuccess: (outcomes: DeleteSessionResult[]) => {
          if (outcomes.some((outcome) => outcome.success)) {
            onSessionDeleted();
          }
          const failures = outcomes.filter((outcome) => !outcome.success);
          if (failures.length > 0) {
            const details = failures
              .slice(0, 3)
              .map((outcome) => outcome.error || "Unknown error")
              .join("\n");
            window.alert(`${failures.length} sessions failed to delete.\n${details}`);
          }
        },
        // Tauri invoke rejects with raw strings, not Error instances.
        onError: (error) => window.alert(invocationErrorMessage(error)),
      });
    },
    [deleteSessionsMutation, onSessionDeleted],
  );

  // ─── Destination-conflict handling (single-session) ────────────────
  // The backend refuses archive/restore when the destination name already
  // exists. On that sentinel error, ask once; confirming resends the same
  // mutation variables with overwrite: true. Cancelling stays silent, the
  // same convention as cancelling a native dialog elsewhere in the app.
  //
  // Both handlers go through mutateAsync + promise chaining instead of
  // mutate(variables, { callbacks }): mutate-call callbacks live on the
  // MutationObserver, so a second mutate() on the same observer while
  // pending, or an unmount, replaces or detaches them; the mutateAsync
  // promise is observer-independent, so the confirm/retry chain and the
  // completion callbacks survive those observer transitions.
  //
  // Rejection reasons from Tauri invoke are raw strings, so every error
  // here goes through invocationErrorMessage instead of error.message.
  const confirmOverwriteAndResend = useCallback(
    async (
      error: unknown,
      input: DeleteSessionOptions,
      resend: (input: DeleteSessionOptions) => void,
    ) => {
      const message = invocationErrorMessage(error);
      if (!isDestinationConflict(message)) {
        window.alert(message);
        return;
      }
      const replace = await confirm(
        "A session already exists at the destination. Replace it?",
        { title: "Destination conflict", kind: "warning" },
      );
      if (replace) {
        resend({ ...input, overwrite: true });
      }
    },
    [],
  );

  const handleRestore = useCallback(
    (session: SessionMeta) => {
      const input = getLifecycleOperationOptions(session);
      if (!input) return;
      const resend = (retry: DeleteSessionOptions) =>
        restoreMutation.mutateAsync(retry).then(
          () => onSessionRestored(),
          (retryError: unknown) => window.alert(invocationErrorMessage(retryError)),
        );
      restoreMutation.mutateAsync(input).then(
        () => onSessionRestored(),
        (error: unknown) => void confirmOverwriteAndResend(error, input, resend),
      );
    },
    [restoreMutation, onSessionRestored, confirmOverwriteAndResend],
  );

  const handleArchive = useCallback(
    (session: SessionMeta) => {
      const input = getLifecycleOperationOptions(session);
      if (!input) return;
      const resend = (retry: DeleteSessionOptions) =>
        archiveMutation.mutateAsync(retry).then(
          () => onSessionArchived(),
          (retryError: unknown) => window.alert(invocationErrorMessage(retryError)),
        );
      archiveMutation.mutateAsync(input).then(
        () => onSessionArchived(),
        (error: unknown) => void confirmOverwriteAndResend(error, input, resend),
      );
    },
    [archiveMutation, onSessionArchived, confirmOverwriteAndResend],
  );

  // ─── Folder-level handlers ────────────────────────────────────────
  const executeFolderOperation = useCallback(
    (action: FolderLifecycleAction, items: DeleteSessionOptions[]) => {
      if (items.length === 0) return;
      const mutate = action === "archive" ? archiveSessionsMutation : restoreSessionsMutation;
      // mutateAsync for the same observer-callback hazard as the
      // single-session handlers: mutate-call callbacks live on the
      // MutationObserver and are replaced or detached by a second mutate()
      // while pending or by an unmount; the mutateAsync promise is
      // observer-independent, so the batch results feeding the
      // conflict-escalation flow survive those observer transitions.
      mutate.mutateAsync(items).then(
        (outcomes) => onFolderOperationComplete(outcomes, action),
        (error: unknown) => window.alert(invocationErrorMessage(error)),
      );
    },
    [archiveSessionsMutation, restoreSessionsMutation, onFolderOperationComplete],
  );

  // ─── Star / Pin handlers ──────────────────────────────────────────
  const handleToggleStar = useCallback(
    (session: SessionMeta) => {
      const key = getMetadataKey(session);
      const currentlyStarred = starredMap.has(key);
      setStarredMutation.mutate({ sessionKey: key, starred: !currentlyStarred });
    },
    [starredMap, setStarredMutation],
  );

  const handleTogglePin = useCallback(
    (folder: string) => {
      const next = pinnedFolders.includes(folder)
        ? pinnedFolders.filter((f) => f !== folder)
        : [...pinnedFolders, folder];
      setPinnedFoldersMutation.mutate(next);
    },
    [pinnedFolders, setPinnedFoldersMutation],
  );

  // ─── Return ───────────────────────────────────────────────────────
  return {
    confirmDeleteSession,
    executeBatchDelete,
    handleArchive,
    handleRestore,
    getFolderOperationItems,
    executeFolderOperation,
    handleToggleStar,
    handleTogglePin,
    isDeletePending: deleteMutation.isPending,
    isArchivePending: archiveMutation.isPending,
    isRestorePending: restoreMutation.isPending,
    isFolderOperationPending: archiveSessionsMutation.isPending || restoreSessionsMutation.isPending,
  };
}
