import { useCallback, useEffect, useMemo, useRef } from "react";
import { getSessionFromMap } from "@/lib/session-map";
import { useSessionMutations } from "@/hooks/useSessionMutations";
import { useSessionQueries } from "@/hooks/useSessionQueries";
import { useSessionUIState } from "@/hooks/useSessionUIState";
import { useUpdater } from "@/hooks/useUpdater";
import type { DeleteSessionResult } from "@/lib/api/sessions";
import type { SessionMeta } from "@/types";
import { getMetadataKey, getSessionKey } from "@/lib/domain";
import { ConfirmDeleteDialog } from "./ConfirmDeleteDialog";
import { FolderFilter } from "./FolderFilter";
import { SessionDetail } from "./SessionDetail";
import { SessionList } from "./SessionList";

export function SessionManagerPage() {
  const ui = useSessionUIState();
  const queries = useSessionQueries(ui.scope, ui.selectedFolder, ui.selectedKey, ui.search);
  const updater = useUpdater();

  // ─── Folder operation result handler ──────────────────────────────
  const handleFolderOperationResult = useCallback(
    (outcomes: DeleteSessionResult[]) => {
      const successes = outcomes.filter((o) => o.success).length;
      const failures = outcomes.filter((o) => !o.success);

      if (successes > 0) {
        ui.setSelectedKey(null);
        ui.setSelectedFolder("all");
      }

      if (failures.length > 0) {
        const details = failures
          .slice(0, 3)
          .map((f) => f.error || "Unknown error")
          .join("\n");
        window.alert(`${failures.length} sessions failed to move.\n${details}`);
      }
    },
    [ui.setSelectedKey, ui.setSelectedFolder],
  );

  const mutations = useSessionMutations(
    queries.sessions,
    queries.pinnedFolders,
    queries.starredMap,
    {
      onSessionDeleted: () => {
        ui.setSelectedKey(null);
        ui.setSessionPendingDelete(null);
      },
      onSessionArchived: () => ui.setSelectedKey(null),
      onSessionRestored: () => ui.setSelectedKey(null),
      onFolderOperationComplete: handleFolderOperationResult,
    },
  );

  // ─── Star filter: sessions visible in the list ──────────────────────
  const displaySessions = useMemo(
    () =>
      ui.showStarredOnly
        ? queries.filteredSessions.filter((s) => queries.starredMap.has(getMetadataKey(s)))
        : queries.filteredSessions,
    [queries.filteredSessions, queries.starredMap, ui.showStarredOnly],
  );

  // Keep a ref to always read the latest selected keys, avoiding handleBatchDelete re-creation on selectedSessionKeys change
  const selectedKeysRef = useRef(ui.selectedSessionKeys);
  selectedKeysRef.current = ui.selectedSessionKeys;

  const selectedKeysSet = useMemo(
    () => new Set(ui.selectedSessionKeys),
    [ui.selectedSessionKeys],
  );

  const handleBatchDelete = useCallback(() => {
    const keys = selectedKeysRef.current;
    const items = keys
      .map((key) => getSessionFromMap(queries.sessionMap, key))
      .filter((s): s is SessionMeta & { sourcePath: string } => Boolean(s?.sourcePath))
      .map((s) => ({
        providerId: s.providerId,
        sessionId: s.sessionId,
        sourcePath: s.sourcePath,
      }));
    mutations.handleBatchDelete(items);
    ui.clearSelection();
  }, [queries.sessionMap, mutations.handleBatchDelete, ui.clearSelection]);

  // ─── Auto-select first available session ──────────────────────────
  useEffect(() => {
    if (ui.selectedKey && displaySessions.some((s) => getSessionKey(s) === ui.selectedKey)) {
      return;
    }
    ui.setSelectedKey(
      displaySessions[0] ? getSessionKey(displaySessions[0]) : null,
    );
  }, [displaySessions, ui.selectedKey, ui.setSelectedKey]);

  // ─── Selection / fork jump handlers ───────────────────────────────
  const handleSelect = useCallback(
    (session: SessionMeta) => {
      ui.setSelectedKey(getSessionKey(session));
      ui.setForkJumpIndex(undefined);
    },
    [ui.setSelectedKey, ui.setForkJumpIndex],
  );

  const handleForkJump = useCallback(
    (session: SessionMeta, forkAtUser: number) => {
      ui.setSelectedKey(getSessionKey(session));
      ui.setForkJumpIndex(forkAtUser);
    },
    [ui.setSelectedKey, ui.setForkJumpIndex],
  );

  // Delete: just sets the pending-delete state; confirmDeleteSession (from mutations) does the actual deletion
  const handleDelete = useCallback(
    (session: SessionMeta) => {
      if (!session.sourcePath) return;
      ui.setSessionPendingDelete(session);
    },
    [ui.setSessionPendingDelete],
  );

  const handleConfirmDelete = useCallback(
    (session: SessionMeta) => {
      ui.setSessionPendingDelete(null); // close dialog immediately, avoid "Deleting..." flicker
      mutations.confirmDeleteSession(session);
    },
    [ui.setSessionPendingDelete, mutations.confirmDeleteSession],
  );

  const handleCancelDelete = useCallback(
    () => ui.setSessionPendingDelete(null),
    [ui.setSessionPendingDelete],
  );

  const handleRefresh = useCallback(() => {
    void queries.sessionsQuery.refetch();
    if (ui.viewMode === "tree") {
      void queries.forkTreeQuery.refetch();
    }
  }, [queries.sessionsQuery.refetch, queries.forkTreeQuery.refetch, ui.viewMode]);

  return (
    <div
      className={`app-shell${ui.isFolderColumnCollapsed ? " no-folder-column" : ""}${ui.viewMode === "tree" ? " tree-mode" : ""}`}
    >
      <FolderFilter
        folders={queries.folderGroups}
        selectedFolder={ui.selectedFolder}
        onSelectFolder={ui.setSelectedFolder}
        pinnedFolders={queries.pinnedFolders}
        onTogglePin={mutations.handleTogglePin}
        isCollapsed={ui.isFolderColumnCollapsed}
        onToggleCollapse={ui.toggleFolderColumn}
        scope={ui.scope}
        onScopeChange={ui.setScope}
        onArchiveFolder={mutations.handleArchiveFolder}
        onRestoreFolder={mutations.handleRestoreFolder}
        isFolderOperationPending={mutations.isFolderOperationPending}
        updateStatus={updater.status}
        updateVersion={updater.update?.version}
        onInstallUpdate={updater.installUpdate}
        onRetryUpdate={updater.retryCheck}
      />
      <SessionList
        sessions={displaySessions}
        sessionMap={queries.sessionMap}
        selectedKey={ui.selectedKey}
        search={ui.search}
        isLoading={queries.sessionsQuery.isLoading}
        error={queries.sessionsQuery.error}
        starredMap={queries.starredMap}
        onSearchChange={ui.setSearch}
        onRefresh={handleRefresh}
        isRefreshing={queries.sessionsQuery.isFetching}
        onSelect={handleSelect}
        scope={ui.scope}
        showStarredOnly={ui.showStarredOnly}
        onToggleStarFilter={ui.toggleStarFilter}
        viewMode={ui.viewMode}
        onToggleViewMode={ui.toggleViewMode}
        treeRoots={queries.treeRoots}
        treeTotalSessions={queries.treeTotalSessions}
        isTreeLoading={queries.forkTreeQuery.isLoading}
        treeError={queries.forkTreeQuery.error}
        onForkJump={handleForkJump}
        selectionMode={ui.selectionMode}
        selectedKeysSet={selectedKeysSet}
        onToggleSelectionMode={ui.toggleSelectionMode}
        onToggleSessionSelection={ui.toggleSessionSelection}
        onBatchDelete={handleBatchDelete}
      />
      <SessionDetail
        session={queries.selectedSession}
        messages={queries.sessionDetailQuery.data?.messages ?? []}
        qaPairs={queries.sessionDetailQuery.data?.qaPairs ?? []}
        rawContent={queries.sessionDetailQuery.data?.rawContent ?? null}
        isLoading={queries.sessionDetailQuery.isLoading}
        error={queries.sessionDetailQuery.error}
        isDeleting={mutations.isDeletePending}
        isArchiving={mutations.isArchivePending}
        isRestoring={mutations.isRestorePending}
        isStarred={queries.isStarred}
        scope={ui.scope}
        onToggleStar={mutations.handleToggleStar}
        onDelete={handleDelete}
        onArchive={mutations.handleArchive}
        onRestore={mutations.handleRestore}
        forkJumpIndex={ui.forkJumpIndex}
      />
      {ui.sessionPendingDelete ? (
        <ConfirmDeleteDialog
          session={ui.sessionPendingDelete}
          isDeleting={false}
          onConfirm={handleConfirmDelete}
          onCancel={handleCancelDelete}
        />
      ) : null}
    </div>
  );
}
