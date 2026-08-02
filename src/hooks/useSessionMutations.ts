import { useCallback } from "react";
import { type DeleteSessionOptions, type DeleteSessionResult } from "@/lib/api/sessions";
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

interface UseSessionMutationsOptions {
  onSessionDeleted: () => void;
  onSessionArchived: () => void;
  onSessionRestored: () => void;
  onFolderOperationComplete: (outcomes: DeleteSessionResult[]) => void;
}

/**
 * Mutation handlers for the session manager.
 * Receives data dependencies (sessions, pinnedFolders, starredMap) and callbacks for UI state updates,
 * keeping the hook free of direct UI state management.
 */
export function useSessionMutations(
  sessions: SessionMeta[],
  pinnedFolders: string[],
  starredMap: Map<string, boolean>,
  options: UseSessionMutationsOptions,
) {
  const { onSessionDeleted, onSessionArchived, onSessionRestored, onFolderOperationComplete } = options;

  // ─── Raw mutations ────────────────────────────────────────────────
  const deleteMutation = useDeleteSessionMutation();
  const deleteSessionsMutation = useDeleteSessionsMutation();
  const archiveMutation = useArchiveSessionMutation();
  const restoreMutation = useRestoreSessionMutation();
  const archiveSessionsMutation = useArchiveSessionsMutation();
  const restoreSessionsMutation = useRestoreSessionsMutation();
  const setPinnedFoldersMutation = useSetPinnedFoldersMutation();
  const setStarredMutation = useSetSessionStarredMutation();

  // ─── Data helpers ─────────────────────────────────────────────────
  const getFolderOperationItems = useCallback(
    (folder: string): { items: DeleteSessionOptions[]; skippedCount: number } => {
      if (folder === "all" || folder === "Unknown") return { items: [], skippedCount: 0 };
      const folderSessions = sessions
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

  const handleArchive = useCallback(
    (session: SessionMeta) => {
      const input = getLifecycleOperationOptions(session);
      if (!input) return;
      archiveMutation.mutate(
        input,
        {
          onSuccess: () => onSessionArchived(),
          onError: (error) => window.alert(error.message),
        },
      );
    },
    [archiveMutation, onSessionArchived],
  );

  const handleRestore = useCallback(
    (session: SessionMeta) => {
      const input = getLifecycleOperationOptions(session);
      if (!input) return;
      restoreMutation.mutate(
        input,
        {
          onSuccess: () => onSessionRestored(),
          onError: (error) => window.alert(error.message),
        },
      );
    },
    [restoreMutation, onSessionRestored],
  );

  // ─── Folder-level handlers ────────────────────────────────────────
  const handleFolderAction = useCallback(
    (folder: string, action: "archive" | "restore") => {
      const { items, skippedCount } = getFolderOperationItems(folder);
      if (items.length === 0) {
        window.alert("No sessions that support this operation were found in this folder.");
        return;
      }

      const verb = action === "archive" ? "Archive" : "Restore";
      const skipped = skippedCount > 0 ? `\n\n${skippedCount} read-only session(s) will be skipped.` : "";
      const ok = window.confirm(
        `${verb} ${items.length} sessions from this folder?${skipped}\n\n${folder}\n\nPinned folders are not changed.`,
      );
      if (!ok) return;

      const mutate = action === "archive" ? archiveSessionsMutation : restoreSessionsMutation;
      mutate.mutate(items, {
        onSuccess: onFolderOperationComplete,
        onError: (error) => window.alert(error.message),
      });
    },
    [archiveSessionsMutation, restoreSessionsMutation, getFolderOperationItems, onFolderOperationComplete],
  );

  const handleArchiveFolder = useCallback(
    (folder: string) => handleFolderAction(folder, "archive"),
    [handleFolderAction],
  );

  const handleRestoreFolder = useCallback(
    (folder: string) => handleFolderAction(folder, "restore"),
    [handleFolderAction],
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
    handleArchiveFolder,
    handleRestoreFolder,
    handleToggleStar,
    handleTogglePin,
    isDeletePending: deleteMutation.isPending,
    isArchivePending: archiveMutation.isPending,
    isRestorePending: restoreMutation.isPending,
    isFolderOperationPending: archiveSessionsMutation.isPending || restoreSessionsMutation.isPending,
  };
}
