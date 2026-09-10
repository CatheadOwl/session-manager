import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ToggleRow } from "./ToggleRow";

afterEach(cleanup);

describe("ToggleRow", () => {
  it("renders an accessible switch reflecting the value", () => {
    render(<ToggleRow label="Check for updates automatically" value={false} onChange={() => {}} />);
    const sw = screen.getByRole("switch", { name: "Check for updates automatically" });
    expect(sw).toHaveAttribute("aria-checked", "false");
  });

  it("toggles on click and reports the next value", () => {
    const onChange = vi.fn();
    render(<ToggleRow label="Auto check" value={true} onChange={onChange} />);
    fireEvent.click(screen.getByRole("switch", { name: "Auto check" }));
    expect(onChange).toHaveBeenCalledWith(false);
  });

  it("toggles on Space and Enter (exactly once per press)", () => {
    const onChange = vi.fn();
    render(<ToggleRow label="Auto check" value={false} onChange={onChange} />);
    const sw = screen.getByRole("switch", { name: "Auto check" });
    fireEvent.keyDown(sw, { key: " " });
    fireEvent.keyUp(sw, { key: " " });
    fireEvent.keyDown(sw, { key: "Enter" });
    expect(onChange).toHaveBeenCalledTimes(2);
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("surfaces the inline error on the row", () => {
    render(<ToggleRow label="Auto check" value={false} onChange={() => {}} error="write failed" />);
    expect(screen.getByRole("alert")).toHaveTextContent("write failed");
  });
});
