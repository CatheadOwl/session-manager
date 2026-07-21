import { describe, it, expect } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useContentCollapse } from "./useContentCollapse";
import type { CollapsedContent } from "@/utils/content-collapse";

const collapsible: CollapsedContent = {
  shouldCollapse: true,
  previewText: "PREVIEW",
  fullText: "FULL",
};
const plain: CollapsedContent = {
  shouldCollapse: false,
  previewText: "FULL",
  fullText: "FULL",
};

describe("useContentCollapse", () => {
  it("starts expanded when content is not collapsible", () => {
    const { result } = renderHook(() => useContentCollapse("short", () => plain));

    expect(result.current.shouldCollapse).toBe(false);
    expect(result.current.expanded).toBe(true);
    expect(result.current.displayContent).toBe("FULL");
  });

  it("starts collapsed (showing preview) when content is collapsible", () => {
    const { result } = renderHook(() => useContentCollapse("long", () => collapsible));

    expect(result.current.shouldCollapse).toBe(true);
    expect(result.current.expanded).toBe(false);
    expect(result.current.displayContent).toBe("PREVIEW");
  });

  it("toggle expands to the full text", () => {
    const { result } = renderHook(() => useContentCollapse("long", () => collapsible));

    act(() => result.current.toggle());

    expect(result.current.expanded).toBe(true);
    expect(result.current.displayContent).toBe("FULL");
  });

  it("resets expanded state when the content changes", () => {
    const { result, rerender } = renderHook(
      ({ content }) => useContentCollapse(content, () => collapsible),
      { initialProps: { content: "first" } },
    );

    act(() => result.current.toggle());
    expect(result.current.expanded).toBe(true);

    rerender({ content: "second" });
    expect(result.current.expanded).toBe(false);
    expect(result.current.displayContent).toBe("PREVIEW");
  });
});
