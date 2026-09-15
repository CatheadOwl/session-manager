import { describe, it, expect } from "vitest";
import { extractSystemBlocks } from "./system-blocks";

describe("extractSystemBlocks", () => {
  it("extracts a single block and returns the remaining text", () => {
    const input = "<system-reminder>\nhello world\n</system-reminder>\nbody text";
    const { text, blocks } = extractSystemBlocks(input);

    expect(blocks).toEqual([{ tag: "system-reminder", content: "hello world" }]);
    expect(text).toBe("body text");
  });

  it("supports tags containing spaces", () => {
    const input = "<permissions instructions>\ndo this\n</permissions instructions>\nrest";
    const { text, blocks } = extractSystemBlocks(input);

    expect(blocks).toEqual([{ tag: "permissions instructions", content: "do this" }]);
    expect(text).toBe("rest");
  });

  it("extracts multiple blocks, preserving in-between text", () => {
    const input = "<a>\none\n</a>\nmiddle\n<b>\ntwo\n</b>";
    const { text, blocks } = extractSystemBlocks(input);

    expect(blocks).toEqual([
      { tag: "a", content: "one" },
      { tag: "b", content: "two" },
    ]);
    expect(text).toBe("middle");
  });

  it("keeps multi-line block content (trimmed)", () => {
    const input = "<note>\nline1\nline2\n</note>";
    const { blocks } = extractSystemBlocks(input);

    expect(blocks).toEqual([{ tag: "note", content: "line1\nline2" }]);
  });

  it("returns the text unchanged when there are no blocks", () => {
    const input = "just plain text";
    const { text, blocks } = extractSystemBlocks(input);

    expect(blocks).toEqual([]);
    expect(text).toBe("just plain text");
  });

  it("does not match a block whose opening tag is mid-line", () => {
    const input = "prefix <system-reminder>\nx\n</system-reminder>";
    const { text, blocks } = extractSystemBlocks(input);

    expect(blocks).toEqual([]);
    expect(text).toBe(input);
  });

  it("extracts codex harness blocks whose closing tag is glued to the last content line", () => {
    const input =
      "<app-context>\ncontext body\n</app-context>\n<multi_agent_mode>Do not delegate.</multi_agent_mode>";
    const { text, blocks } = extractSystemBlocks(input);

    expect(blocks).toEqual([
      { tag: "app-context", content: "context body" },
      { tag: "multi_agent_mode", content: "Do not delegate." },
    ]);
    expect(text).toBe("");
  });

  it("matches inline-close blocks to the last closing tag, skipping in-body tag mentions", () => {
    const input =
      "<collaboration_mode># Mode: Default\nChange mode via a new `<collaboration_mode>...</collaboration_mode>` block.\nFollow the mode rules.</collaboration_mode>";
    const { text, blocks } = extractSystemBlocks(input);

    expect(blocks).toEqual([
      {
        tag: "collaboration_mode",
        content:
          "# Mode: Default\nChange mode via a new `<collaboration_mode>...</collaboration_mode>` block.\nFollow the mode rules.",
      },
    ]);
    expect(text).toBe("");
  });

  it("extracts multi_agent_role containing unclosed placeholder tags", () => {
    const input =
      "<multi_agent_role>You are `/root`.\nTask name: <recipient>\nSender: <author>\n<payload text>\nBe legible.</multi_agent_role>";
    const { text, blocks } = extractSystemBlocks(input);

    expect(blocks).toEqual([
      {
        tag: "multi_agent_role",
        content:
          "You are `/root`.\nTask name: <recipient>\nSender: <author>\n<payload text>\nBe legible.",
      },
    ]);
    expect(text).toBe("");
  });

  it("does not extract mid-line inline tag pairs from prose", () => {
    const input = "see <cwd>/tmp</cwd> for details";
    const { text, blocks } = extractSystemBlocks(input);

    expect(blocks).toEqual([]);
    expect(text).toBe(input);
  });
});
