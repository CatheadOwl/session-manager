import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionMeta } from "@/types";
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

  const renderList = (sessions: SessionMeta[]) => {
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
        />
      </QueryClientProvider>,
    );
  };

  const renderSelectionList = (sessions: SessionMeta[], initialSelectedKeys: string[] = []) => {
    const sessionMap = new Map(sessions.map((item) => [getSessionKey(item), item]));
    const hiddenKey = "claude:hidden-session";

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
            />
          </QueryClientProvider>
          <div data-testid="selected-keys">{selectedKeys.join(",")}</div>
          <div data-testid="hidden-key">{hiddenKey}</div>
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

  it("shows indeterminate when some visible sessions are selected and preserves hidden selections", () => {
    const sessions = [session("one"), session("two"), session("three")];
    const initialSelectedKeys = [getSessionKey(sessions[0]), "claude:hidden-session"];

    renderSelectionList(sessions, initialSelectedKeys);

    const checkbox = screen.getByRole("checkbox", {
      name: "Select all visible sessions",
    }) as HTMLInputElement;

    expect(checkbox.checked).toBe(false);
    expect(checkbox.indeterminate).toBe(true);

    fireEvent.click(checkbox);

    expect(checkbox.checked).toBe(true);
    expect(checkbox.indeterminate).toBe(false);
    expect(screen.getByTestId("selected-keys")).toHaveTextContent("claude:hidden-session");
    expect(screen.getByTestId("selected-keys")).toHaveTextContent(getSessionKey(sessions[0]));
    expect(screen.getByTestId("selected-keys")).toHaveTextContent(getSessionKey(sessions[1]));
    expect(screen.getByTestId("selected-keys")).toHaveTextContent(getSessionKey(sessions[2]));
  });
});
