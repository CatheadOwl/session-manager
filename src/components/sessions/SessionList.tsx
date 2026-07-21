import { createContext, memo, useContext, useMemo, useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { RefreshCw, Star } from "lucide-react";
import type { SessionMeta } from "@/types";
import type { TreeNodeData } from "@/lib/api/sessions";
import { SessionItem } from "./SessionItem";
import { TreeView } from "./TreeView";
import { getSessionKey } from "@/lib/domain";

// ─── Selection context (ref-based, avoids re-render on mode toggle) ────────

interface SelectionCtxValue {
  modeRef: React.MutableRefObject<boolean>;
  toggleSessionSelection: (key: string) => void;
}

const SelectionContext = createContext<SelectionCtxValue | null>(null);

export function useSelectionContext() {
  return useContext(SelectionContext);
}

// ─── Component ─────────────────────────────────────────────────────────────

interface SessionListProps {
  sessions: SessionMeta[];
  sessionMap: Map<string, SessionMeta>;
  selectedKey: string | null;
  search: string;
  isLoading: boolean;
  error: Error | null;
  starredMap: Map<string, boolean>;
  onSearchChange: (value: string) => void;
  onRefresh: () => void;
  isRefreshing: boolean;
  onSelect: (session: SessionMeta) => void;
  scope?: "active" | "archived";
  // Star filter
  showStarredOnly: boolean;
  onToggleStarFilter: () => void;
  // Tree view props
  viewMode: "flat" | "tree";
  onToggleViewMode: () => void;
  treeRoots: TreeNodeData[];
  treeTotalSessions: number;
  isTreeLoading: boolean;
  treeError: Error | null;
  onForkJump?: (session: SessionMeta, forkAtUser: number) => void;
  // Selection mode props
  selectionMode: boolean;
  selectedKeysSet: Set<string>;
  onToggleSelectionMode: () => void;
  onToggleSessionSelection: (key: string) => void;
  onBatchDelete: () => void;
}

export const SessionList = memo(function SessionList({
  sessions,
  sessionMap,
  selectedKey,
  search,
  isLoading,
  error,
  starredMap,
  onSearchChange,
  onRefresh,
  isRefreshing,
  onSelect,
  scope = "active",
  showStarredOnly,
  onToggleStarFilter,
  viewMode,
  onToggleViewMode,
  treeRoots,
  treeTotalSessions,
  isTreeLoading,
  treeError,
  onForkJump,
  selectionMode,
  selectedKeysSet,
  onToggleSelectionMode,
  onToggleSessionSelection,
  onBatchDelete,
}: SessionListProps) {
  // Ref-based context: mode toggle won't trigger re-render of context consumers
  const modeRef = useRef(selectionMode);
  modeRef.current = selectionMode;

  const ctxValue = useMemo<SelectionCtxValue>(
    () => ({ modeRef, toggleSessionSelection: onToggleSessionSelection }),
    [onToggleSessionSelection],
  );

  const selectedCount = useMemo(
    () => selectedKeysSet.size,
    [selectedKeysSet],
  );

  const listScrollRef = useRef<HTMLDivElement>(null);

  const listVirtualizer = useVirtualizer({
    count: viewMode === "flat" ? sessions.length : 0,
    getScrollElement: () => listScrollRef.current,
    estimateSize: () => 110,
    overscan: 5,
  });

  return (
    <SelectionContext.Provider value={ctxValue}>
      <aside className={`session-sidebar${selectionMode ? " selection-mode" : ""}`}>
        {/* Row 1: View mode toggle (left) + Refresh (right) */}
        <header className="sidebar-header">
          <div className="sidebar-actions">
            <div className="view-mode-segment" role="group" aria-label="Session view mode">
              <button
                type="button"
                className={`view-mode-option${viewMode === "flat" ? " active" : ""}`}
                onClick={viewMode === "tree" ? onToggleViewMode : undefined}
                aria-pressed={viewMode === "flat"}
                title="List view"
              >
                List
              </button>
              <button
                type="button"
                className={`view-mode-option${viewMode === "tree" ? " active" : ""}`}
                onClick={viewMode === "flat" ? onToggleViewMode : undefined}
                aria-pressed={viewMode === "tree"}
                title="Fork tree view"
              >
                Tree
              </button>
            </div>
            <button
              type="button"
              className={`secondary-button refresh-button${isRefreshing ? " spinning" : ""}`}
              onClick={onRefresh}
              disabled={isRefreshing}
              aria-label="Refresh sessions"
              title="Refresh sessions"
            >
              <RefreshCw size={14} />
            </button>
          </div>
        </header>

        {/* Row 2: Search + Star filter */}
        <div className="search-row">
          <div className="search-row-inner">
            <input
              value={search}
              onChange={(event) => onSearchChange(event.target.value)}
              placeholder="Search sessions..."
              aria-label="Search sessions"
            />
            <button
              type="button"
              className={`star-search-btn${showStarredOnly ? " active" : ""}`}
              onClick={onToggleStarFilter}
              aria-label={showStarredOnly ? "Show all sessions" : "Show starred sessions only"}
              title={showStarredOnly ? "Show all sessions" : "Show starred sessions only"}
            >
              <Star aria-hidden="true" strokeWidth={2.4} />
            </button>
          </div>
        </div>

        {/* Row 3 (list view only): Selection mode */}
        {viewMode === "flat" ? (
          <div className="selection-row">
            <button
              type="button"
              className={`selection-toggle-btn${selectionMode ? " active" : ""}`}
              onClick={onToggleSelectionMode}
            >
              {selectionMode ? "Cancel" : "Select"}
            </button>
            {selectionMode ? (
              <button
                type="button"
                className="batch-delete-btn"
                disabled={selectedCount === 0}
                onClick={onBatchDelete}
              >
                Delete{selectedCount > 0 ? ` (${selectedCount})` : ""}
              </button>
            ) : null}
          </div>
        ) : null}

        {/* Status line */}
        <div className="list-status">
          {viewMode === "tree"
            ? isTreeLoading
              ? "Building fork tree..."
              : `${treeTotalSessions} session(s), ${treeRoots.length} root(s)`
            : isLoading
              ? "Loading sessions..."
              : `${sessions.length} sessions${showStarredOnly ? " (starred)" : ""}${selectionMode ? ` — ${selectedCount} selected` : ""}`
          }
        </div>
        {error ? <div className="error-box">{error.message}</div> : null}

        <div className="session-list" ref={listScrollRef}>
          {viewMode === "tree" ? (
            <>
              {treeError ? <div className="error-box">{treeError.message}</div> : null}
              {isTreeLoading ? (
                <div className="empty-state">
                  <strong>Computing fork tree...</strong>
                  <span>Analyzing user event sequences to discover session relationships.</span>
                </div>
              ) : (
                <TreeView
                  roots={treeRoots}
                  sessionMap={sessionMap}
                  selectedKey={selectedKey}
                  search={search}
                  starredMap={starredMap}
                  onSelect={onSelect}
                  onForkJump={onForkJump}
                  starFilterActive={showStarredOnly}
                />
              )}
            </>
          ) : (
            <>
              {!isLoading && sessions.length === 0 ? (
                <div className="empty-state">
                  <strong>No sessions found</strong>
                  <span>
                    {scope === "active"
                      ? "Check that your AI coding agent's session directory contains sessions."
                      : "No archived sessions found."}
                  </span>
                </div>
              ) : null}
              {sessions.length > 0 && (
                <div style={{ height: `${listVirtualizer.getTotalSize()}px`, position: "relative" }}>
                  {listVirtualizer.getVirtualItems().map((virtualRow) => {
                    const session = sessions[virtualRow.index];
                    const key = getSessionKey(session);
                    return (
                      <div
                        key={key}
                        data-index={virtualRow.index}
                        ref={listVirtualizer.measureElement}
                        style={{
                          position: "absolute",
                          top: 0,
                          left: 0,
                          width: "100%",
                          transform: `translateY(${virtualRow.start}px)`,
                          paddingBottom: "0.65rem",
                        }}
                      >
                        <SessionItem
                          session={session}
                          selected={selectedKey === key}
                          search={search}
                          onSelect={() => onSelect(session)}
                          isSelectionSelected={selectedKeysSet.has(key)}
                        />
                      </div>
                    );
                  })}
                </div>
              )}
            </>
          )}
        </div>
      </aside>
    </SelectionContext.Provider>
  );
});
