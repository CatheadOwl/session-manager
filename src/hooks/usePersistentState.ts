import { useCallback, useState } from "react";

/**
 * Like useState, but persists the value to localStorage.
 * On mount, reads the stored value (falls back to defaultValue).
 * Every set writes through to localStorage synchronously.
 */
export function usePersistentState<T>(
  key: string,
  defaultValue: T,
): [T, (value: T | ((prev: T) => T)) => void] {
  const [state, setState] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(key);
      if (raw !== null) return JSON.parse(raw) as T;
    } catch {
      // ignore parse errors, fall through to default
    }
    return defaultValue;
  });

  const set = useCallback(
    (value: T | ((prev: T) => T)) => {
      setState((prev) => {
        const next =
          typeof value === "function" ? (value as (p: T) => T)(prev) : value;
        try {
          localStorage.setItem(key, JSON.stringify(next));
        } catch {
          // storage full or unavailable — state still updates in-memory
        }
        return next;
      });
    },
    [key],
  );

  return [state, set];
}
