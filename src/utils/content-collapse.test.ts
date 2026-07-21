import { describe, it, expect } from "vitest";
import { getAssistantPreview, getLegacyCollapsedMessage } from "./content-collapse";

describe("getAssistantPreview", () => {
  it("does not collapse empty content", () => {
    const r = getAssistantPreview("");
    expect(r.shouldCollapse).toBe(false);
    expect(r.previewText).toBe("");
  });

  it("does not collapse short content", () => {
    const r = getAssistantPreview("short text");
    expect(r.shouldCollapse).toBe(false);
    expect(r.previewText).toBe("short text");
    expect(r.fullText).toBe("short text");
  });

  it("collapses long multi-line content into head + ellipsis + tail", () => {
    const content = Array.from({ length: 10 }, (_, i) => `line${i + 1}`).join("\n");
    const r = getAssistantPreview(content);

    expect(r.shouldCollapse).toBe(true);
    expect(r.fullText).toBe(content);
    expect(r.previewText).toBe("line1\nline2\nline3\n\n...\n\nline8\nline9\nline10");
  });

  it("does not collapse long content that has too few lines (edge guard)", () => {
    // 300 chars on a single line: long by length but only 1 line.
    const content = "x".repeat(300);
    const r = getAssistantPreview(content);

    expect(r.shouldCollapse).toBe(false);
    expect(r.previewText).toBe(content);
  });

  it("normalizes CRLF to LF", () => {
    const content = "a\r\nb\r\nc";
    const r = getAssistantPreview(content);
    expect(r.fullText).toBe("a\nb\nc");
  });
});

describe("getLegacyCollapsedMessage", () => {
  it("does not collapse content within the threshold", () => {
    const content = "y".repeat(3000);
    const r = getLegacyCollapsedMessage(content);

    expect(r.shouldCollapse).toBe(false);
    expect(r.previewText).toBe(content);
  });

  it("collapses content over the threshold to a 1500-char preview", () => {
    const content = "z".repeat(4000);
    const r = getLegacyCollapsedMessage(content);

    expect(r.shouldCollapse).toBe(true);
    expect(r.previewText).toBe("z".repeat(1500) + "...");
    expect(r.fullText).toBe(content);
  });
});
