import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionDetail, SessionMeta } from "@/types";
import { SessionManagerPage } from "./SessionManagerPage";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn().mockResolvedValue(null),
}));

vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: vi.fn(),
}));

vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 200,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        key: index,
        start: index * 200,
        size: 200,
      })),
    measureElement: vi.fn(),
    scrollToIndex: vi.fn(),
  }),
}));

vi.mock("./FolderFilter", () => ({
  FolderFilter: () => null,
}));

vi.mock("./SessionList", () => ({
  SessionList: ({ sessions, onSelect }: { sessions: SessionMeta[]; onSelect: (session: SessionMeta) => void }) => (
    <div>
      {sessions.map((session) => (
        <button key={session.sessionId} type="button" onClick={() => onSelect(session)}>
          {session.title}
        </button>
      ))}
    </div>
  ),
}));

describe("SessionManagerPage", () => {
  let queryClient: QueryClient;

  const session = (
    sessionId: string,
    title: string,
    recordId = sessionId,
  ): SessionMeta => ({
    providerId: "opencode",
    sessionId,
    title,
    projectDir: "/tmp/opencode",
    sourcePath: "/data/opencode.db",
    locator: { kind: "database", path: "/data/opencode.db", recordId },
  });

  const detail = (content: string): SessionDetail => ({
    messages: [{ role: "user", content, ts: 1 }],
    qaPairs: [],
  });

  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    mocks.invoke.mockReset();
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });
  });

  it("updates MessagesSection when selecting a different OpenCode DB session", async () => {
    const sessions = [
      session("ses_a", "Session A"),
      session("ses_b", "Session B"),
    ];
    const detailsById = new Map([
      ["ses_a", detail("message from A")],
      ["ses_b", detail("message from B")],
    ]);

    mocks.invoke.mockImplementation((command: string, payload?: { sessionId?: string }) => {
      if (command === "list_sessions") return Promise.resolve(sessions);
      if (command === "get_app_metadata") return Promise.resolve({ sessions: {}, pinnedFolders: [] });
      if (command === "compute_fork_tree") {
        return Promise.resolve({
          roots: [],
          totalSessions: 0,
          computedFromCache: false,
          durationMs: 0,
        });
      }
      if (command === "get_session_detail") {
        return Promise.resolve(detailsById.get(payload?.sessionId ?? "") ?? detail("unknown"));
      }
      return Promise.resolve(null);
    });

    render(<SessionManagerPage />, { wrapper });

    await screen.findByRole("button", { name: "Session A" });
    await waitFor(() => {
      expect(screen.getByText("message from A")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: "Session B" }));

    await waitFor(() => {
      expect(screen.getByText("message from B")).toBeInTheDocument();
    });
    expect(screen.queryByText("message from A")).not.toBeInTheDocument();
  });
});
