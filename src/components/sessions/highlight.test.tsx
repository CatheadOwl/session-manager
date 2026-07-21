import { describe, it, expect, afterEach } from "vitest";
import { render, cleanup } from "@testing-library/react";
import { highlightText } from "./highlight";

afterEach(() => cleanup());

const renderToContainer = (text: string, query: string) =>
  render(<>{highlightText(text, query)}</>);

describe("highlightText", () => {
  it("returns the raw text when the query is empty", () => {
    expect(highlightText("hello", "")).toBe("hello");
    expect(highlightText("hello", "   ")).toBe("hello");
  });

  it("wraps a case-insensitive match in a <mark>", () => {
    const { container } = renderToContainer("Hello World", "world");

    const mark = container.querySelector("mark.highlight");
    expect(mark).not.toBeNull();
    expect(mark?.textContent).toBe("World");
    expect(container.textContent).toBe("Hello World");
  });

  it("highlights all occurrences", () => {
    const { container } = renderToContainer("aba", "a");
    expect(container.querySelectorAll("mark.highlight")).toHaveLength(2);
    expect(container.textContent).toBe("aba");
  });

  it("leaves text intact when there is no match", () => {
    const { container } = renderToContainer("hello", "xyz");
    expect(container.querySelectorAll("mark")).toHaveLength(0);
    expect(container.textContent).toBe("hello");
  });
});
