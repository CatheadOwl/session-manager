import { describe, it, expect } from "vitest";
import { renderHook } from "@testing-library/react";
import { useSessionSearch } from "./useSessionSearch";
import type { SessionMeta } from "@/types";

const sessions: SessionMeta[] = [
  { providerId: "claude", sessionId: "c1", title: "Auth refactor", lastActiveAt: 100 },
  { providerId: "claude", sessionId: "c2", title: "Bug fix", lastActiveAt: 300 },
  { providerId: "gemini", sessionId: "g1", title: "Auth docs", lastActiveAt: 200 },
];

const ids = (list: SessionMeta[]) => list.map((s) => s.sessionId);

describe("useSessionSearch", () => {
  it("returns all sessions sorted by recency on an empty query", () => {
    const { result } = renderHook(() => useSessionSearch({ sessions, providerFilter: "all" }));
    expect(ids(result.current.search(""))).toEqual(["c2", "g1", "c1"]);
  });

  it("falls back to createdAt when lastActiveAt is missing", () => {
    const list: SessionMeta[] = [
      { providerId: "claude", sessionId: "a", createdAt: 50 },
      { providerId: "claude", sessionId: "b", lastActiveAt: 100 },
    ];
    const { result } = renderHook(() => useSessionSearch({ sessions: list, providerFilter: "all" }));
    expect(ids(result.current.search("  "))).toEqual(["b", "a"]);
  });

  it("matches by title via the full-text index", () => {
    const { result } = renderHook(() => useSessionSearch({ sessions, providerFilter: "all" }));
    const found = ids(result.current.search("auth")).sort();
    expect(found).toEqual(["c1", "g1"]);
  });

  it("returns nothing for a query that matches no session", () => {
    const { result } = renderHook(() => useSessionSearch({ sessions, providerFilter: "all" }));
    expect(result.current.search("zzz")).toEqual([]);
  });

  it("restricts both indexing and results to the provider filter", () => {
    const { result } = renderHook(() => useSessionSearch({ sessions, providerFilter: "claude" }));

    // Empty query → only claude sessions, recency-sorted.
    expect(ids(result.current.search(""))).toEqual(["c2", "c1"]);

    // "auth" matches c1 (claude) and g1 (gemini), but gemini is filtered out.
    expect(ids(result.current.search("auth"))).toEqual(["c1"]);
  });
});
