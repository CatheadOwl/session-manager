import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionMeta } from "@/types";
import type { TimeRange } from "@/utils/time-range";
import { getSessionKey } from "@/lib/domain";
import { SessionList } from "./SessionList";

const mocks = vi.hoisted(() => ({
  useVirtualizer: vi.fn(),
}));

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: mocks.useVirtualizer,
}));

describe("SessionList", () => {
  let queryClient: QueryClient;

  const session = (sessionId: string, summary?: string): SessionMeta => ({
    providerId: "claude",
    sessionId,
    title: `Session ${sessionId}`,
    summary,
    sourcePath: `/tmp/${sessionId}.jsonl`,
    createdAt: 1,
  });

  const remoteSession = (sessionId: string): SessionMeta => ({
    ...session(sessionId),
    locator: { kind: "remote", sourceId: "ali-server", path: `/home/u/${sessionId}.jsonl` },
  });

  const renderList = (sessions: SessionMeta[], timeRange: TimeRange = { preset: "all" }) => {
    const sessionMap = new Map(sessions.map((item) => [getSessionKey(item), item]));
    return render(
      <QueryClientProvider client={queryClient}>
        <SessionList
          sessions={sessions}
          sessionMap={sessionMap}
          selectedKey={null}
          search=""
          isLoading={false}
          error={null}
          starredMap={new Map()}
          onSearchChange={vi.fn()}
          onRefresh={vi.fn()}
          isRefreshing={false}
          onSelect={vi.fn()}
          showStarredOnly={false}
          onToggleStarFilter={vi.fn()}
          viewMode="flat"
          onToggleViewMode={vi.fn()}
          treeRoots={[]}
          treeTotalSessions={0}
          isTreeLoading={false}
          treeError={null}
          selectionMode={false}
          selectedKeysSet={new Set()}
          onToggleSelectionMode={vi.fn()}
          onToggleSessionSelection={vi.fn()}
          visibleSessionKeys={sessions.map(getSessionKey)}
          onSelectSessionKeys={vi.fn()}
          onUnselectSessionKeys={vi.fn()}
          onBatchDelete={vi.fn()}
          exportSessions={sessions}
          timeRange={timeRange}
          onTimeRangePresetChange={vi.fn()}
          onCustomTimeRange={vi.fn()}
          onExportQa={vi.fn()}
          exportStatus={{ state: "idle", message: "" }}
        />
      </QueryClientProvider>,
    );
  };

  const renderSelectionList = (sessions: SessionMeta[], initialSelectedKeys: string[] = []) => {
    const sessionMap = new Map(sessions.map((item) => [getSessionKey(item), item]));

    function Harness() {
      const [selectedKeys, setSelectedKeys] = useState(initialSelectedKeys);

      const handleSelectKeys = (keys: string[]) => {
        setSelectedKeys((prev) => {
          const next = [...prev];
          const seen = new Set(prev);
          for (const key of keys) {
            if (seen.has(key)) continue;
            seen.add(key);
            next.push(key);
          }
          return next;
        });
      };

      const handleUnselectKeys = (keys: string[]) => {
        const remove = new Set(keys);
        setSelectedKeys((prev) => prev.filter((key) => !remove.has(key)));
      };

      return (
        <>
          <QueryClientProvider client={queryClient}>
            <SessionList
              sessions={sessions}
              sessionMap={sessionMap}
              selectedKey={null}
              search=""
              isLoading={false}
              error={null}
              starredMap={new Map()}
              onSearchChange={vi.fn()}
              onRefresh={vi.fn()}
              isRefreshing={false}
              onSelect={vi.fn()}
              showStarredOnly={false}
              onToggleStarFilter={vi.fn()}
              viewMode="flat"
              onToggleViewMode={vi.fn()}
              treeRoots={[]}
              treeTotalSessions={0}
              isTreeLoading={false}
              treeError={null}
              selectionMode={true}
              selectedKeysSet={new Set(selectedKeys)}
              onToggleSelectionMode={vi.fn()}
              onToggleSessionSelection={vi.fn()}
              visibleSessionKeys={sessions.map(getSessionKey)}
              onSelectSessionKeys={handleSelectKeys}
              onUnselectSessionKeys={handleUnselectKeys}
              onBatchDelete={vi.fn()}
              exportSessions={sessions.filter((item) => selectedKeys.includes(getSessionKey(item)))}
              timeRange={{ preset: "all" }}
              onTimeRangePresetChange={vi.fn()}
              onCustomTimeRange={vi.fn()}
              onExportQa={vi.fn()}
              exportStatus={{ state: "idle", message: "" }}
            />
          </QueryClientProvider>
          <div data-testid="selected-keys">{selectedKeys.join(",")}</div>
        </>
      );
    }

    return render(<Harness />);
  };

  beforeEach(() => {
    cleanup();
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    mocks.useVirtualizer.mockReset();
    mocks.useVirtualizer.mockImplementation(({ count }) => ({
      getTotalSize: () => count * 100,
      getVirtualItems: () =>
        Array.from({ length: count }, (_, index) => ({
          index,
          key: index,
          start: index * 100,
          size: 100,
        })),
      measureElement: vi.fn(),
    }));
  });

  it("keys virtual rows by session identity so dynamic height measurements follow reordered sessions", () => {
    const sessions = [
      session("with-summary", "Preview text"),
      session("title-only"),
    ];

    renderList(sessions);

    expect(mocks.useVirtualizer).toHaveBeenCalledWith(
      expect.objectContaining({
        count: sessions.length,
        getItemKey: expect.any(Function),
      }),
    );
    const options = mocks.useVirtualizer.mock.calls[0][0];
    expect(options.getItemKey(0)).toBe(getSessionKey(sessions[0]));
    expect(options.getItemKey(1)).toBe(getSessionKey(sessions[1]));
    expect(screen.getByRole("button", { name: /Session title-only/i })).toBeInTheDocument();
  });

  it("renders a tristate bulk checkbox and toggles only the visible sessions", () => {
    const sessions = [session("one"), session("two"), session("three")];

    renderSelectionList(sessions);

    const checkbox = screen.getByRole("checkbox", {
      name: "Select all visible sessions",
    }) as HTMLInputElement;

    expect(checkbox.checked).toBe(false);
    expect(checkbox.indeterminate).toBe(false);

    fireEvent.click(checkbox);

    expect(checkbox.checked).toBe(true);
    expect(checkbox.indeterminate).toBe(false);
    expect(screen.getByTestId("selected-keys")).toHaveTextContent(
      `${getSessionKey(sessions[0])},${getSessionKey(sessions[1])},${getSessionKey(sessions[2])}`,
    );

    fireEvent.click(checkbox);

    expect(checkbox.checked).toBe(false);
    expect(checkbox.indeterminate).toBe(false);
    expect(screen.getByTestId("selected-keys")).toHaveTextContent("");
  });

  it("hover on the export button states the exportable count and remote exclusions", () => {
    const sessions = [session("one"), session("two"), remoteSession("srv")];

    renderList(sessions, { preset: "7d" });

    const title = screen
      .getByRole("button", { name: /Export Q&A/i })
      .getAttribute("title");
    expect(title).toContain("2 session(s)");
    expect(title).toContain("+1 remote, excluded");
    expect(title).not.toContain("selected in this view");
  });

  it("export hover in selection mode states the selection-narrowed count", () => {
    const sessions = [session("one"), session("two")];

    renderSelectionList(sessions, [getSessionKey(sessions[0])]);

    const title = screen
      .getByRole("button", { name: /Export Q&A/i })
      .getAttribute("title");
    expect(title).toContain("1 session(s) selected");
    expect(title).not.toContain("outside this view");
  });

  it("export button is disabled with a hint when selection mode has nothing checked", () => {
    const sessions = [session("one")];

    renderSelectionList(sessions);

    const button = screen.getByRole("button", { name: /Export Q&A/i }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(button.getAttribute("title")).toContain("select sessions first");
  });

  it("shows indeterminate when some visible sessions are selected, then selects the rest", () => {
    const sessions = [session("one"), session("two"), session("three")];
    const initialSelectedKeys = [getSessionKey(sessions[0])];

    renderSelectionList(sessions, initialSelectedKeys);

    const checkbox = screen.getByRole("checkbox", {
      name: "Select all visible sessions",
    }) as HTMLInputElement;

    expect(checkbox.checked).toBe(false);
    expect(checkbox.indeterminate).toBe(true);

    fireEvent.click(checkbox);

    expect(checkbox.checked).toBe(true);
    expect(checkbox.indeterminate).toBe(false);
    expect(screen.getByTestId("selected-keys")).toHaveTextContent(
      `${getSessionKey(sessions[0])},${getSessionKey(sessions[1])},${getSessionKey(sessions[2])}`,
    );
  });
});
