import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { UpdateToast } from "./UpdateToast";

// No vitest globals → testing-library auto-cleanup is not registered; do it explicitly.
afterEach(() => cleanup());

describe("UpdateToast", () => {
  it("renders nothing when idle", () => {
    const { container } = render(<UpdateToast status="idle" onInstall={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });

  // Regression: a failed check must stay silent (no red dot / button),
  // so users are not misled into thinking an update is available.
  it("renders nothing when error (silent by design)", () => {
    const { container } = render(<UpdateToast status="error" onInstall={vi.fn()} />);
    expect(container.firstChild).toBeNull();
  });

  it("shows a disabled spinning button while checking", () => {
    render(<UpdateToast status="checking" onInstall={vi.fn()} />);
    const btn = screen.getByRole("button");

    expect(btn).toBeDisabled();
    expect(btn.className).toContain("spinning");
    expect(btn.getAttribute("title")).toBe("Checking for updates…");
  });

  it("shows a disabled spinning button while downloading, without a dot", () => {
    const { container } = render(<UpdateToast status="downloading" onInstall={vi.fn()} />);
    const btn = screen.getByRole("button");

    expect(btn).toBeDisabled();
    expect(btn.className).toContain("spinning");
    expect(container.querySelector(".update-dot")).toBeNull();
  });

  it("shows an actionable button with a red dot when available", () => {
    const onInstall = vi.fn();
    const { container } = render(
      <UpdateToast status="available" version="0.2.0" onInstall={onInstall} />,
    );
    const btn = screen.getByRole("button");

    expect(btn).not.toBeDisabled();
    expect(btn.getAttribute("title")).toBe("Update to v0.2.0");
    expect(container.querySelector(".update-dot")).not.toBeNull();

    fireEvent.click(btn);
    expect(onInstall).toHaveBeenCalledTimes(1);
  });
});
