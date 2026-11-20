import { useCallback } from "react";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
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
          onError: (error) => window.alert(error.message),
        },
      );
    },
    [deleteMutation, onSessionDeleted],
  );

  const executeBatchDelete = useCallback(
    (sessionsToDelete: DeleteSessionOptions[]) => {
      if (sessionsToDelete.length === 0) return;

      deleteSessionsMutation.mutate(sessionsToDelete, {
        onSuccess: () => {
          onSessionDeleted();
        },
        onError: (error) => window.alert(error.message),
      });
    },
    [deleteSessionsMutation, onSessionDeleted],
  );

  // ─── Destination-conflict handling (single-session) ────────────────
  // The backend refuses archive/restore when the destination name already
  // exists. On that sentinel error, ask once; confirming resends the same
  // mutation variables with overwrite: true. Cancelling stays silent, the
  // same convention as cancelling a native dialog elsewhere in the app.
  const confirmOverwriteAndResend = useCallback(
    async (
      error: Error,
      input: DeleteSessionOptions,
      resend: (input: DeleteSessionOptions) => void,
    ) => {
      if (!isDestinationConflict(error.message)) {
        window.alert(error.message);
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

  const handleArchive = useCallback(
    (session: SessionMeta) => {
      const input = getLifecycleOperationOptions(session);
      if (!input) return;
      const resend = (retry: DeleteSessionOptions) =>
        archiveMutation.mutate(retry, {
          onSuccess: () => onSessionArchived(),
          onError: (retryError) => window.alert(retryError.message),
        });
      archiveMutation.mutate(
        input,
        {
          onSuccess: () => onSessionArchived(),
          onError: (error) => void confirmOverwriteAndResend(error, input, resend),
        },
      );
    },
    [archiveMutation, onSessionArchived, confirmOverwriteAndResend],
  );

  const handleRestore = useCallback(
    (session: SessionMeta) => {
      const input = getLifecycleOperationOptions(session);
      if (!input) return;
      const resend = (retry: DeleteSessionOptions) =>
        restoreMutation.mutate(retry, {
          onSuccess: () => onSessionRestored(),
          onError: (retryError) => window.alert(retryError.message),
        });
      restoreMutation.mutate(
        input,
        {
          onSuccess: () => onSessionRestored(),
          onError: (error) => void confirmOverwriteAndResend(error, input, resend),
        },
      );
    },
    [restoreMutation, onSessionRestored, confirmOverwriteAndResend],
  );

  // ─── Folder-level handlers ────────────────────────────────────────
  const executeFolderOperation = useCallback(
    (action: FolderLifecycleAction, items: DeleteSessionOptions[]) => {
      if (items.length === 0) return;
      const mutate = action === "archive" ? archiveSessionsMutation : restoreSessionsMutation;
      mutate.mutate(items, {
        onSuccess: (outcomes) => onFolderOperationComplete(outcomes, action),
        onError: (error) => window.alert(error.message),
      });
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
