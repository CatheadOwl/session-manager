import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import type { FolderGroup } from "@/lib/domain";
import { FolderFilter } from "./FolderFilter";

afterEach(cleanup);

const folders: FolderGroup[] = [
  { name: "alpha", count: 1, lastActiveAt: 100 },
  { name: "beta", count: 1, lastActiveAt: 300 },
  { name: "gamma", count: 1, lastActiveAt: 200 },
  { name: "delta", count: 1, lastActiveAt: 250 },
];

// Insertion order (gamma, alpha, beta) differs from both recency (beta, gamma, alpha)
// and alphabetical (alpha, beta, gamma) order, so each sort is genuinely exercised.
const baseProps = {
  selectedFolder: "all",
  onSelectFolder: () => {},
  onTogglePin: () => {},
  isCollapsed: false,
  onToggleCollapse: () => {},
  scope: "active" as const,
  onScopeChange: () => {},
  onArchiveFolder: () => {},
  onRestoreFolder: () => {},
  isFolderOperationPending: false,
  updateStatus: "idle" as const,
  onInstallUpdate: () => {},
};

const renderFilter = (overrides: Partial<Parameters<typeof FolderFilter>[0]> = {}) =>
  render(
    <FolderFilter
      folders={folders}
      pinnedFolders={["gamma", "alpha", "beta"]}
      {...baseProps}
      {...overrides}
    />,
  );

const folderNames = (container: HTMLElement) =>
  Array.from(container.querySelectorAll(".folder-item-name")).map((el) => el.textContent);

describe("FolderFilter sorting", () => {
  it("defaults to recent-first sort applied independently to each section", () => {
    const { container } = renderFilter();
    // Pinned re-sorts by recency (beta 300 > gamma 200 > alpha 100); delta is the
    // only unpinned folder, so it follows under the Folders section.
    expect(folderNames(container)).toEqual(["All", "beta", "gamma", "alpha", "delta"]);
  });

  it("sorts alphabetically within each section when A-Z is selected", () => {
    const { container } = renderFilter();
    fireEvent.click(screen.getByRole("button", { name: /^Sort folders/ }));
    fireEvent.click(screen.getByRole("menuitemradio", { name: /^A-Z$/ }));
    expect(folderNames(container)).toEqual(["All", "alpha", "beta", "gamma", "delta"]);
  });

  it("breaks recency ties alphabetically", () => {
    const { container } = renderFilter({
      folders: [
        { name: "omega", count: 1, lastActiveAt: 100 },
        { name: "zeta", count: 1, lastActiveAt: 100 },
      ],
      pinnedFolders: [],
    });
    expect(folderNames(container)).toEqual(["All", "omega", "zeta"]);
  });

  it("omits the Pinned section label when no folders are pinned", () => {
    renderFilter({ pinnedFolders: [] });
    expect(screen.queryByText("Pinned")).toBeNull();
  });

  it("shows the current sort as the trigger label", () => {
    renderFilter();
    const trigger = screen.getByRole("button", { name: /^Sort folders/ });
    expect(trigger.textContent).toContain("Recent");
    fireEvent.click(trigger);
    fireEvent.click(screen.getByRole("menuitemradio", { name: /^A-Z$/ }));
    expect(trigger.textContent).toContain("A-Z");
  });
});
