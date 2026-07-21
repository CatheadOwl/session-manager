import { useCallback, useMemo, useState } from "react";
import type { SessionMessage } from "@/types";

interface UseMessageSearchOptions {
  messages: SessionMessage[];
  enabled: boolean;
}

interface UseMessageSearchResult {
  query: string;
  setQuery: (q: string) => void;
  /** Indices (into messages array) of matching messages, in order. */
  matchIndices: number[];
  /** Position within matchIndices of the current match, -1 if none. */
  currentMatch: number;
  /** Message index of the current match, -1 if none. */
  currentMsgIndex: number;
  goNext: () => void;
  goPrev: () => void;
  clear: () => void;
}

/**
 * Find-in-page style search over session messages.
 * Case-insensitive substring match against message content.
 */
export function useMessageSearch({ messages, enabled }: UseMessageSearchOptions): UseMessageSearchResult {
  const [query, setQuery] = useState("");
  const [currentMatch, setCurrentMatch] = useState(-1);

  const matchIndices = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle || !enabled) return [];
    const result: number[] = [];
    for (let i = 0; i < messages.length; i++) {
      if (messages[i].content.toLowerCase().includes(needle)) {
        result.push(i);
      }
    }
    return result;
  }, [messages, query, enabled]);

  // Clamp currentMatch when matches change
  const clamped = matchIndices.length === 0 ? -1 : Math.min(currentMatch, matchIndices.length - 1);

  const setQueryAndReset = useCallback((q: string) => {
    setQuery(q);
    setCurrentMatch(q.trim() ? 0 : -1);
  }, []);

  const goNext = useCallback(() => {
    if (matchIndices.length === 0) return;
    setCurrentMatch((prev) => (prev + 1) % matchIndices.length);
  }, [matchIndices.length]);

  const goPrev = useCallback(() => {
    if (matchIndices.length === 0) return;
    setCurrentMatch((prev) => (prev - 1 + matchIndices.length) % matchIndices.length);
  }, [matchIndices.length]);

  const clear = useCallback(() => {
    setQuery("");
    setCurrentMatch(-1);
  }, []);

  return {
    query,
    setQuery: setQueryAndReset,
    matchIndices,
    currentMatch: clamped,
    currentMsgIndex: clamped >= 0 ? matchIndices[clamped] : -1,
    goNext,
    goPrev,
    clear,
  };
}
