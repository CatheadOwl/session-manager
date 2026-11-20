import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MarkdownContent } from "./MarkdownContent";

const mocks = vi.hoisted(() => ({
  openUrl: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: mocks.openUrl,
}));

beforeEach(() => {
  mocks.openUrl.mockReset();
  mocks.openUrl.mockResolvedValue(undefined);
});

afterEach(cleanup);

describe("MarkdownContent links", () => {
  it.each([
    ["https://example.com/docs", "[docs](https://example.com/docs)"],
    ["http://example.com", "<http://example.com>"],
    ["mailto:someone@example.com", "[mail](mailto:someone@example.com)"],
  ])("opens %s in the system handler", (href, markdown) => {
    render(<MarkdownContent content={markdown} />);
    const link = screen.getByRole("link");

    const notCancelled = fireEvent.click(link);

    expect(notCancelled).toBe(false);
    expect(mocks.openUrl).toHaveBeenCalledWith(href);
  });

  it("does not navigate the app window or open non-web links", () => {
    render(<MarkdownContent content="[file](src/lib/domain.ts)" />);

    const notCancelled = fireEvent.click(screen.getByRole("link"));

    expect(notCancelled).toBe(false);
    expect(mocks.openUrl).not.toHaveBeenCalled();
  });

  it("does not use target=_blank, which the webview ignores", () => {
    render(<MarkdownContent content="[docs](https://example.com)" />);

    expect(screen.getByRole("link")).not.toHaveAttribute("target");
  });

  // react-markdown's urlTransform strips hrefs outside http/https/irc/mailto/xmpp,
  // so these render as bare anchors with no href — the click gate never sees them.
  // If this starts failing, revisit the EXTERNAL_LINK gate before widening it.
  it.each(["javascript:alert(1)", "tel:+123456"])(
    "renders no anchor for a %s link",
    (href) => {
      render(<MarkdownContent content={`[x](${href})`} />);

      expect(screen.queryByRole("link")).toBeNull();
      expect(mocks.openUrl).not.toHaveBeenCalled();
    },
  );
});
