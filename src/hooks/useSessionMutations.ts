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
import { getMetadataKey } from "@/lib/domain";

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
    (folder: string): DeleteSessionOptions[] => {
      if (folder === "all" || folder === "Unknown") return [];
      return sessions
        .filter((session) => normalizeProjectDir(session.projectDir) === folder)
        .filter((session): session is SessionMeta & { sourcePath: string } => Boolean(session.sourcePath))
        .map((session) => ({
          providerId: session.providerId,
          sessionId: session.sessionId,
          sourcePath: session.sourcePath,
        }));
    },
    [sessions],
  );

  // ─── Single-session handlers ──────────────────────────────────────
  const confirmDeleteSession = useCallback(
    (session: SessionMeta) => {
      if (!session.sourcePath) return;

      deleteMutation.mutate(
        {
          providerId: session.providerId,
          sessionId: session.sessionId,
          sourcePath: session.sourcePath,
        },
        {
          onSuccess: () => onSessionDeleted(),
          onError: (error) => window.alert(error.message),
        },
      );
    },
    [deleteMutation, onSessionDeleted],
  );

  const handleBatchDelete = useCallback(
    (sessionsToDelete: { providerId: string; sessionId: string; sourcePath: string }[]) => {
      if (sessionsToDelete.length === 0) return;

      const ok = window.confirm(`Delete ${sessionsToDelete.length} selected session(s)?\n\nThis cannot be undone.`);
      if (!ok) return;

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
      if (!session.sourcePath) return;
      archiveMutation.mutate(
        {
          providerId: session.providerId,
          sessionId: session.sessionId,
          sourcePath: session.sourcePath,
        },
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
      if (!session.sourcePath) return;
      restoreMutation.mutate(
        {
          providerId: session.providerId,
          sessionId: session.sessionId,
          sourcePath: session.sourcePath,
        },
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
      const items = getFolderOperationItems(folder);
      if (items.length === 0) {
        window.alert("No sessions with source files were found in this folder.");
        return;
      }

      const verb = action === "archive" ? "Archive" : "Restore";
      const ok = window.confirm(
        `${verb} ${items.length} sessions from this folder?\n\n${folder}\n\nPinned folders are not changed.`,
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
    handleBatchDelete,
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
