import { describe, it, expect } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useMessageSearch } from "./useMessageSearch";
import type { SessionMessage } from "@/types";

const messages: SessionMessage[] = [
  { role: "user", content: "Hello world" },
  { role: "assistant", content: "Hi there" },
  { role: "user", content: "hello again" },
];

const setup = (enabled = true) =>
  renderHook(() => useMessageSearch({ messages, enabled }));

describe("useMessageSearch", () => {
  it("has no matches before any query", () => {
    const { result } = setup();
    expect(result.current.matchIndices).toEqual([]);
    expect(result.current.currentMatch).toBe(-1);
    expect(result.current.currentMsgIndex).toBe(-1);
  });

  it("finds case-insensitive substring matches", () => {
    const { result } = setup();
    act(() => result.current.setQuery("HELLO"));

    expect(result.current.matchIndices).toEqual([0, 2]);
    expect(result.current.currentMatch).toBe(0);
    expect(result.current.currentMsgIndex).toBe(0);
  });

  it("returns no matches when disabled", () => {
    const { result } = setup(false);
    act(() => result.current.setQuery("hello"));

    expect(result.current.matchIndices).toEqual([]);
    expect(result.current.currentMsgIndex).toBe(-1);
  });

  it("returns no matches for a query that hits nothing", () => {
    const { result } = setup();
    act(() => result.current.setQuery("zzz"));

    expect(result.current.matchIndices).toEqual([]);
    expect(result.current.currentMatch).toBe(-1);
  });

  it("goNext advances and wraps around", () => {
    const { result } = setup();
    act(() => result.current.setQuery("hello")); // match 0

    act(() => result.current.goNext());
    expect(result.current.currentMsgIndex).toBe(2); // match 1

    act(() => result.current.goNext());
    expect(result.current.currentMsgIndex).toBe(0); // wrapped to match 0
  });

  it("goPrev moves backward and wraps around", () => {
    const { result } = setup();
    act(() => result.current.setQuery("hello")); // match 0

    act(() => result.current.goPrev());
    expect(result.current.currentMsgIndex).toBe(2); // wrapped to last match
  });

  it("clear resets the query and position", () => {
    const { result } = setup();
    act(() => result.current.setQuery("hello"));
    act(() => result.current.clear());

    expect(result.current.query).toBe("");
    expect(result.current.currentMatch).toBe(-1);
    expect(result.current.matchIndices).toEqual([]);
  });

  it("clamps the position when the match set shrinks", () => {
    const { result } = setup();
    act(() => result.current.setQuery("hello")); // [0, 2]
    act(() => result.current.goNext()); // position 1

    // Narrow the query so only one message matches.
    act(() => result.current.setQuery("again")); // [2]
    expect(result.current.matchIndices).toEqual([2]);
    expect(result.current.currentMatch).toBe(0);
    expect(result.current.currentMsgIndex).toBe(2);
  });
});
