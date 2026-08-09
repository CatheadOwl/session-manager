import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import type { PropsWithChildren } from "react";
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
          onBatchDelete={vi.fn()}
        />
      </QueryClientProvider>,
    );
  };

  beforeEach(() => {
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
});
