import { cleanup, render, screen, fireEvent } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Menu, MenuItem } from "./Menu";

afterEach(cleanup);

function renderMenu() {
  render(
    <Menu
      label="Test menu"
      renderTrigger={(props) => <button type="button" {...props}>Open</button>}
    >
      <MenuItem>First</MenuItem>
      <MenuItem>Second</MenuItem>
      <MenuItem disabled>Disabled</MenuItem>
    </Menu>,
  );
}

describe("Menu keyboard navigation", () => {
  it("ArrowDown from the trigger focuses the first enabled item", () => {
    renderMenu();
    const trigger = screen.getByRole("button", { name: "Test menu" });
    fireEvent.click(trigger);
    fireEvent.keyDown(trigger, { key: "ArrowDown" });

    expect(document.activeElement).toBe(screen.getByRole("menuitem", { name: "First" }));
  });

  it("ArrowDown wraps from the last item to the first and skips disabled items", () => {
    renderMenu();
    const trigger = screen.getByRole("button", { name: "Test menu" });
    fireEvent.click(trigger);
    const panel = screen.getByRole("menu");
    const last = screen.getByText("Second");
    last.focus();
    fireEvent.keyDown(panel, { key: "ArrowDown" });

    expect(document.activeElement).toBe(screen.getByText("First"));
  });

  it("ArrowUp focuses the last item when no item is focused", () => {
    renderMenu();
    const trigger = screen.getByRole("button", { name: "Test menu" });
    fireEvent.click(trigger);
    const panel = screen.getByRole("menu");
    panel.focus();
    fireEvent.keyDown(panel, { key: "ArrowUp" });

    expect(document.activeElement).toBe(screen.getByText("Second"));
  });

  it("Escape closes the menu and restores focus to the trigger", () => {
    const onBlur = vi.fn();
    render(
      <Menu
        label="Esc menu"
        renderTrigger={(props) => <button type="button" {...props}>Open</button>}
      >
        <MenuItem onClick={onBlur}>Only</MenuItem>
      </Menu>,
    );
    const trigger = screen.getByRole("button", { name: "Esc menu" });
    fireEvent.click(trigger);
    const panel = screen.getByRole("menu");
    fireEvent.keyDown(panel, { key: "Escape" });

    expect(screen.queryByRole("menu")).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("Home focuses the first item", () => {
    renderMenu();
    const trigger = screen.getByRole("button", { name: "Test menu" });
    fireEvent.click(trigger);
    const panel = screen.getByRole("menu");
    screen.getByText("Second").focus();
    fireEvent.keyDown(panel, { key: "Home" });

    expect(document.activeElement).toBe(screen.getByText("First"));
  });
});
