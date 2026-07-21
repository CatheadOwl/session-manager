import { useCallback, useEffect, useState } from "react";
import { usePersistentState } from "@/hooks/usePersistentState";
import type { SessionMeta } from "@/types";

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
  const [forkJumpIndex, setForkJumpIndex] = useState<number | undefined>(undefined);
  const [sessionPendingDelete, setSessionPendingDelete] = useState<SessionMeta | null>(null);
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

  const clearSelection = useCallback(() => {
    setSelectedSessionKeys([]);
    setSelectionMode(false);
  }, []);

  // Reset folder selection when switching scope
  useEffect(() => {
    setSelectedFolder("all");
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
    forkJumpIndex,
    setForkJumpIndex,
    sessionPendingDelete,
    setSessionPendingDelete,
    selectionMode,
    toggleSelectionMode,
    selectedSessionKeys,
    toggleSessionSelection,
    clearSelection,
  };
}
