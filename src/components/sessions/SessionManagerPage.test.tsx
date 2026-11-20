import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionDetail, SessionMeta } from "@/types";
import { confirm } from "@tauri-apps/plugin-dialog";
import { getSessionKey } from "@/lib/domain";
import type { MockInstance } from "vitest";
import { SessionManagerPage } from "./SessionManagerPage";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  saveDialog: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: vi.fn().mockResolvedValue(true),
  save: mocks.saveDialog,
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
  FolderFilter: ({
    onArchiveFolder,
    onRestoreFolder,
  }: {
    onArchiveFolder?: (folder: string) => void;
    onRestoreFolder?: (folder: string) => void;
  }) => (
    <div>
      {/* Folder archive/restore entry for lifecycle-flow tests; the real
          column renders per-folder rows. The folder matches the fixture
          projectDir below (POSIX path passes through normalizeProjectDir). */}
      <button type="button" onClick={() => onArchiveFolder?.("/tmp/opencode")}>
        Archive Folder
      </button>
      <button type="button" onClick={() => onRestoreFolder?.("/tmp/opencode")}>
        Restore Folder
      </button>
    </div>
  ),
}));

vi.mock("./SessionList", () => ({
  SessionList: ({
    sessions,
    onSelect,
    onRefresh,
    onSearchChange,
    onToggleSelectionMode,
    onToggleSessionSelection,
    onBatchDelete,
    onExportQa,
  }: {
    sessions: SessionMeta[];
    onSelect: (session: SessionMeta) => void;
    onRefresh: () => void;
    onSearchChange: (value: string) => void;
    onToggleSelectionMode: () => void;
    onToggleSessionSelection: (key: string) => void;
    onBatchDelete: () => void;
    onExportQa: () => void;
  }) => (
    <div>
      <button type="button" onClick={onRefresh}>
        Refresh
      </button>
      <button type="button" onClick={onToggleSelectionMode}>
        Select
      </button>
      <button type="button" onClick={onBatchDelete}>
        Delete Selected
      </button>
      <button type="button" onClick={onExportQa}>
        Export Q&A
      </button>
      <input aria-label="Search" onChange={(event) => onSearchChange(event.target.value)} />
      {sessions.map((session) => (
        <span key={session.sessionId}>
          <button type="button" onClick={() => onSelect(session)}>
            {session.title}
          </button>
          <input
            type="checkbox"
            aria-label={`Check ${session.title}`}
            onChange={() => onToggleSessionSelection(getSessionKey(session))}
          />
        </span>
      ))}
    </div>
  ),
}));

describe("SessionManagerPage", () => {
  let queryClient: QueryClient;
  // Scoped here so afterEach restores it even when an assertion in the
  // middle of a test throws.
  let alertSpy: MockInstance;

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
    vi.mocked(confirm).mockClear();
    alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });
  });

  afterEach(() => {
    cleanup();
    alertSpy.mockRestore();
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
      if (command === "get_app_metadata") return Promise.resolve({ sessions: {}, pinned_folders: [] });
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

  it("refreshes the selected session detail without switching selection", async () => {
    const sessions = [session("ses_a", "Session A")];
    const detailResponses = [detail("old message"), detail("new message")];
    let detailCalls = 0;

    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_sessions") return Promise.resolve(sessions);
      if (command === "get_app_metadata") return Promise.resolve({ sessions: {}, pinned_folders: [] });
      if (command === "compute_fork_tree") {
        return Promise.resolve({
          roots: [],
          totalSessions: 0,
          computedFromCache: false,
          durationMs: 0,
        });
      }
      if (command === "get_session_detail") {
        const response = detailResponses[Math.min(detailCalls, detailResponses.length - 1)];
        detailCalls += 1;
        return Promise.resolve(response);
      }
      return Promise.resolve(null);
    });

    render(<SessionManagerPage />, { wrapper });

    await screen.findByRole("button", { name: "Session A" });
    await waitFor(() => {
      expect(screen.getByText("old message")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    await waitFor(() => {
      expect(screen.getByText("new message")).toBeInTheDocument();
    });
    expect(screen.queryByText("old message")).not.toBeInTheDocument();
    expect(detailCalls).toBe(2);
  });

  it("exports only the checked visible sessions when selection mode is on", async () => {
    const sessions = [
      session("ses_a", "Session A"),
      session("ses_b", "Session B"),
    ];

    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_sessions") return Promise.resolve(sessions);
      if (command === "get_app_metadata") return Promise.resolve({ sessions: {}, pinned_folders: [] });
      if (command === "get_session_detail") return Promise.resolve(detail("message"));
      if (command === "compute_fork_tree") {
        return Promise.resolve({
          roots: [],
          totalSessions: 0,
          computedFromCache: false,
          durationMs: 0,
        });
      }
      if (command === "export_qa_sessions") {
        return Promise.resolve({ count: 1, skipped: [], destPath: "/tmp/qa-export.json" });
      }
      return Promise.resolve(null);
    });
    mocks.saveDialog.mockResolvedValue("/tmp/qa-export.json");

    render(<SessionManagerPage />, { wrapper });

    await screen.findByRole("button", { name: "Session A" });

    fireEvent.click(screen.getByRole("button", { name: "Select" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Check Session A" }));
    fireEvent.click(screen.getByRole("button", { name: "Export Q&A" }));

    await waitFor(() => {
      const call = mocks.invoke.mock.calls.find(([command]) => command === "export_qa_sessions");
      expect(call).toBeDefined();
      const options = call![1]?.options as { sessions?: SessionMeta[] };
      expect(options.sessions).toHaveLength(1);
      expect(options.sessions![0].sessionId).toBe("ses_a");
    });
  });

  it("re-scopes the checked set when the visible list narrows (search)", async () => {
    const sessions = [
      session("ses_a", "Alpha report"),
      session("ses_b", "Beta report"),
    ];

    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_sessions") return Promise.resolve(sessions);
      if (command === "get_app_metadata") return Promise.resolve({ sessions: {}, pinned_folders: [] });
      if (command === "get_session_detail") return Promise.resolve(detail("message"));
      if (command === "compute_fork_tree") {
        return Promise.resolve({
          roots: [],
          totalSessions: 0,
          computedFromCache: false,
          durationMs: 0,
        });
      }
      if (command === "export_qa_sessions") {
        return Promise.resolve({ count: 1, skipped: [], destPath: "/tmp/qa-export.json" });
      }
      return Promise.resolve(null);
    });
    mocks.saveDialog.mockResolvedValue("/tmp/qa-export.json");

    render(<SessionManagerPage />, { wrapper });

    await screen.findByRole("button", { name: "Alpha report" });

    fireEvent.click(screen.getByRole("button", { name: "Select" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Check Alpha report" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Check Beta report" }));

    // Narrow: Alpha falls out of view, its check must be dropped.
    fireEvent.change(screen.getByRole("textbox", { name: "Search" }), {
      target: { value: "Beta" },
    });
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "Alpha report" })).not.toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Export Q&A" }));

    // Widen back: Alpha is visible again but stays unchecked — the check
    // was dropped, not parked. A second export still covers Beta only.
    fireEvent.change(screen.getByRole("textbox", { name: "Search" }), {
      target: { value: "" },
    });
    await screen.findByRole("button", { name: "Alpha report" });
    fireEvent.click(screen.getByRole("button", { name: "Export Q&A" }));

    await waitFor(() => {
      expect(
        mocks.invoke.mock.calls.filter(([command]) => command === "export_qa_sessions"),
      ).toHaveLength(2);
    });
    const exports = mocks.invoke.mock.calls.filter(
      ([command]) => command === "export_qa_sessions",
    );
    for (const call of exports) {
      const options = call[1]?.options as { sessions?: SessionMeta[] };
      expect(options.sessions?.map((s) => s.sessionId)).toEqual(["ses_b"]);
    }
  });

  it("batch delete targets only the checked sessions still visible after the list narrows", async () => {
    // File-backed sessions: database-locator sessions are read-only and
    // never enter a delete payload.
    const sessions: SessionMeta[] = [
      { ...session("ses_a", "Alpha report"), locator: { kind: "file", path: "/tmp/alpha.jsonl" } },
      { ...session("ses_b", "Beta report"), locator: { kind: "file", path: "/tmp/beta.jsonl" } },
    ];

    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_sessions") return Promise.resolve(sessions);
      if (command === "get_app_metadata") return Promise.resolve({ sessions: {}, pinned_folders: [] });
      if (command === "get_session_detail") return Promise.resolve(detail("message"));
      if (command === "compute_fork_tree") {
        return Promise.resolve({
          roots: [],
          totalSessions: 0,
          computedFromCache: false,
          durationMs: 0,
        });
      }
      return Promise.resolve(null);
    });

    render(<SessionManagerPage />, { wrapper });

    await screen.findByRole("button", { name: "Alpha report" });

    fireEvent.click(screen.getByRole("button", { name: "Select" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Check Alpha report" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "Check Beta report" }));

    // Narrow: Alpha falls out of view, its check must be dropped from the
    // delete set too — the confirm dialog must count 1, not 2.
    fireEvent.change(screen.getByRole("textbox", { name: "Search" }), {
      target: { value: "Beta" },
    });
    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "Alpha report" })).not.toBeInTheDocument();
    });
    fireEvent.click(screen.getByRole("button", { name: "Delete Selected" }));

    expect(screen.getByText("Delete 1 session?")).toBeInTheDocument();
  });

  it("merges destination conflicts into one grouped confirm and resends only the conflict subset", async () => {
    const sessions: SessionMeta[] = ["ses_ok", "ses_conflict", "ses_conflict2", "ses_failed"].map((id) => ({
      providerId: "opencode",
      sessionId: id,
      title: id,
      projectDir: "/tmp/opencode",
      sourcePath: `/tmp/${id}.jsonl`,
      locator: { kind: "file", path: `/tmp/${id}.jsonl` },
    }));
    // ADR 0014 Decision 3: the expected literals are independent of the
    // production constants — a prefix drift on either side must fail here.
    const conflictError = "Destination conflict: another session already exists at the destination";
    const ordinaryError = "Failed to move session file: access denied";
    const outcome = (sessionId: string, success: boolean, error?: string) => ({
      providerId: "opencode",
      sessionId,
      sourcePath: `/tmp/${sessionId}.jsonl`,
      locator: { kind: "file", path: `/tmp/${sessionId}.jsonl` },
      success,
      ...(error === undefined ? {} : { error }),
    });
    let archiveBatches = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_sessions") return Promise.resolve(sessions);
      if (command === "get_app_metadata") return Promise.resolve({ sessions: {}, pinned_folders: [] });
      if (command === "get_session_detail") return Promise.resolve(detail("message"));
      if (command === "compute_fork_tree") {
        return Promise.resolve({ roots: [], totalSessions: 0, computedFromCache: false, durationMs: 0 });
      }
      if (command === "archive_sessions") {
        archiveBatches += 1;
        if (archiveBatches === 1) {
          return Promise.resolve([
            outcome("ses_ok", true),
            outcome("ses_conflict", false, conflictError),
            outcome("ses_conflict2", false, conflictError),
            outcome("ses_failed", false, ordinaryError),
          ]);
        }
        // The overwrite retry cannot conflict again.
        return Promise.resolve([outcome("ses_conflict", true), outcome("ses_conflict2", true)]);
      }
      return Promise.resolve(null);
    });

    render(<SessionManagerPage />, { wrapper });

    await screen.findByRole("button", { name: "ses_ok" });

    fireEvent.click(screen.getByRole("button", { name: "Archive Folder" }));
    expect(screen.getByText("Archive 4 sessions?")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    // One grouped confirm for the whole conflict set — not one per item.
    // Two conflict items pin this: a per-item confirm loop fails the call
    // count, and a merged confirm with a wrong count fails the message.
    await waitFor(() => expect(vi.mocked(confirm)).toHaveBeenCalledTimes(1));
    expect(vi.mocked(confirm)).toHaveBeenCalledWith(
      "2 session(s) already exist at the destination. Replace them?",
      { title: "Destination conflict", kind: "warning" },
    );

    // The retry lane resends exactly the conflict subset with overwrite.
    await waitFor(() => expect(archiveBatches).toBe(2));
    const payloads = mocks.invoke.mock.calls
      .filter(([command]) => command === "archive_sessions")
      .map(([, payload]) => payload.items);
    expect(payloads[1]).toHaveLength(2);
    expect(payloads[1].map((item: { sessionId: string }) => item.sessionId)).toEqual([
      "ses_conflict",
      "ses_conflict2",
    ]);
    expect(payloads[1].every((item: { overwrite?: boolean }) => item.overwrite === true)).toBe(true);

    // Ordinary failure alerts separately with the full report — the retry
    // outcome re-enters the same handler and must not double-report.
    await waitFor(() => expect(alertSpy).toHaveBeenCalledTimes(1));
    expect(alertSpy.mock.calls[0][0]).toBe("1 sessions failed to move.\nFailed to move session file: access denied");
    expect(vi.mocked(confirm)).toHaveBeenCalledTimes(1);
  });

  it("does not treat a near-miss prefix as a destination conflict", async () => {
    const sessions: SessionMeta[] = ["ses_a", "ses_b"].map((id) => ({
      providerId: "opencode",
      sessionId: id,
      title: id,
      projectDir: "/tmp/opencode",
      sourcePath: `/tmp/${id}.jsonl`,
      locator: { kind: "file", path: `/tmp/${id}.jsonl` },
    }));
    // Starts with "Destination conflict" but lacks the colon — the contract's
    // exact literal must not match, or near-miss failures would silently skip
    // the ordinary failure alert.
    const nearMissError = "Destination conflict session already exists";
    const ordinaryError = "Failed to move session file: access denied";
    let archiveBatches = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_sessions") return Promise.resolve(sessions);
      if (command === "get_app_metadata") return Promise.resolve({ sessions: {}, pinned_folders: [] });
      if (command === "get_session_detail") return Promise.resolve(detail("message"));
      if (command === "compute_fork_tree") {
        return Promise.resolve({ roots: [], totalSessions: 0, computedFromCache: false, durationMs: 0 });
      }
      if (command === "archive_sessions") {
        archiveBatches += 1;
        return Promise.resolve([
          { providerId: "opencode", sessionId: "ses_a", sourcePath: "/tmp/ses_a.jsonl", locator: { kind: "file", path: "/tmp/ses_a.jsonl" }, success: false, error: nearMissError },
          { providerId: "opencode", sessionId: "ses_b", sourcePath: "/tmp/ses_b.jsonl", locator: { kind: "file", path: "/tmp/ses_b.jsonl" }, success: false, error: ordinaryError },
        ]);
      }
      return Promise.resolve(null);
    });

    render(<SessionManagerPage />, { wrapper });

    await screen.findByRole("button", { name: "ses_a" });

    fireEvent.click(screen.getByRole("button", { name: "Archive Folder" }));
    expect(screen.getByText("Archive 2 sessions?")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Archive" }));

    // No grouped confirm and no overwrite resend — both failures take the
    // ordinary alert path in one report.
    await waitFor(() => expect(alertSpy).toHaveBeenCalledTimes(1));
    expect(vi.mocked(confirm)).not.toHaveBeenCalled();
    const archiveBatchesSeen = mocks.invoke.mock.calls.filter(([command]) => command === "archive_sessions").length;
    expect(archiveBatchesSeen).toBe(1);
    expect(alertSpy.mock.calls[0][0]).toBe(
      "2 sessions failed to move.\nDestination conflict session already exists\nFailed to move session file: access denied",
    );
  });
});
