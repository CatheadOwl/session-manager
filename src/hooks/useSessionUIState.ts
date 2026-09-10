import { useCallback, useEffect, useState } from "react";
import { usePersistentState } from "@/hooks/usePersistentState";
import type { SessionMeta } from "@/types";
import { type TimeRange, type TimeRangePreset } from "@/utils/time-range";

export interface PendingFolderOperation {
  folder: string;
  action: "archive" | "restore";
  sessions: SessionMeta[];
}

/**
 * Pure UI state for the session manager: no queries, no mutations, no business logic.
 * All state here is local component state that could be persisted or reset independently.
 */
export function useSessionUIState() {
  const [search, setSearch] = useState("");
  const [selectedKey, setSelectedKey] = useState<string | null>(null);
  const [selectedFolder, setSelectedFolder] = useState<string>("all");
  const [isFolderColumnCollapsed, setIsFolderColumnCollapsed] = useState(false);
  const [scope, setScope] = useState<"active" | "archived">("active");
  const [viewMode, setViewMode] = usePersistentState<"flat" | "tree">("sm:view-mode", "flat");
  const [showStarredOnly, setShowStarredOnly] = useState(false);
  const [timeRange, setTimeRange] = useState<TimeRange>({ preset: "all" });
  const [forkJumpIndex, setForkJumpIndex] = useState<number | undefined>(undefined);
  const [sessionPendingDelete, setSessionPendingDelete] = useState<SessionMeta | null>(null);
  const [batchDeletePending, setBatchDeletePending] = useState<SessionMeta[] | null>(null);
  const [folderOperationPending, setFolderOperationPending] = useState<PendingFolderOperation | null>(null);
  const [selectionMode, setSelectionMode] = useState(false);
  const [selectedSessionKeys, setSelectedSessionKeys] = useState<string[]>([]);

  const toggleFolderColumn = useCallback(() => {
    setIsFolderColumnCollapsed((prev) => !prev);
  }, []);

  const toggleViewMode = useCallback(() => {
    setViewMode((prev) => (prev === "flat" ? "tree" : "flat"));
  }, []);

  const toggleStarFilter = useCallback(() => {
    setShowStarredOnly((prev) => !prev);
  }, []);

  const setTimeRangePreset = useCallback((preset: TimeRangePreset) => {
    setTimeRange({ preset });
  }, []);

  const setCustomTimeRange = useCallback((from: number, to: number) => {
    setTimeRange({ preset: "custom", customFrom: from, customTo: to });
  }, []);

  const toggleSelectionMode = useCallback(() => {
    setSelectionMode((prev) => !prev);
    if (selectionMode) {
      setSelectedSessionKeys([]);
    } else {
      setSelectedKey(null);
    }
  }, [selectionMode]);

  const toggleSessionSelection = useCallback((key: string) => {
    setSelectedSessionKeys((prev) => {
      if (prev.includes(key)) {
        return prev.filter((k) => k !== key);
      }
      return [...prev, key];
    });
  }, []);

  const selectSessionKeys = useCallback((keys: string[]) => {
    if (keys.length === 0) return;
    setSelectedSessionKeys((prev) => {
      const seen = new Set(prev);
      const next = [...prev];
      let changed = false;

      for (const key of keys) {
        if (seen.has(key)) continue;
        seen.add(key);
        next.push(key);
        changed = true;
      }

      return changed ? next : prev;
    });
  }, []);

  const unselectSessionKeys = useCallback((keys: string[]) => {
    if (keys.length === 0) return;
    const remove = new Set(keys);
    setSelectedSessionKeys((prev) => {
      const next = prev.filter((key) => !remove.has(key));
      return next.length === prev.length ? prev : next;
    });
  }, []);

  // Drop every checked key that is not in the retain list. The page runs
  // this whenever the visible list changes, keeping the checked set a
  // subset of the visible list — so every selection consumer (batch
  // delete, Q&A export) operates on one scope and no action can target a
  // session outside the current view.
  const retainSessionKeys = useCallback((retainKeys: string[]) => {
    const keep = new Set(retainKeys);
    setSelectedSessionKeys((prev) => {
      const next = prev.filter((key) => keep.has(key));
      return next.length === prev.length ? prev : next;
    });
  }, []);

  const clearSelection = useCallback(() => {
    setSelectedSessionKeys([]);
    setSelectionMode(false);
  }, []);

  // Reset folder selection when switching scope
  useEffect(() => {
    setSelectedFolder("all");
    setFolderOperationPending(null);
  }, [scope]);

  // Exit selection mode when switching view mode
  useEffect(() => {
    setSelectionMode(false);
    setSelectedSessionKeys([]);
  }, [viewMode]);

  return {
    search,
    setSearch,
    selectedKey,
    setSelectedKey,
    selectedFolder,
    setSelectedFolder,
    isFolderColumnCollapsed,
    toggleFolderColumn,
    scope,
    setScope,
    viewMode,
    toggleViewMode,
    showStarredOnly,
    toggleStarFilter,
    timeRange,
    setTimeRangePreset,
    setCustomTimeRange,
    forkJumpIndex,
    setForkJumpIndex,
    sessionPendingDelete,
    setSessionPendingDelete,
    batchDeletePending,
    setBatchDeletePending,
    folderOperationPending,
    setFolderOperationPending,
    selectionMode,
    toggleSelectionMode,
    selectedSessionKeys,
    toggleSessionSelection,
    selectSessionKeys,
    unselectSessionKeys,
    retainSessionKeys,
    clearSelection,
  };
}
