import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionMeta } from "@/types";
import { useSessionMutations } from "./useSessionMutations";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

const dialogMocks = vi.hoisted(() => ({
  confirm: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  confirm: dialogMocks.confirm,
}));

// A folder with: two file-backed sessions (archivable), one database-backed
// session (skipped by getLifecycleOperationOptions), plus a session in a
// different folder that must never be included in the folder batch.
const folderSession = (
  sessionId: string,
  projectDir = "d:/work/target",
): SessionMeta => ({
  providerId: "claude",
  sessionId,
  projectDir,
  sourcePath: `d:/work/target/${sessionId}.jsonl`,
  locator: { kind: "file", path: `d:/work/target/${sessionId}.jsonl` },
});

const sessions: SessionMeta[] = [
  folderSession("a"),
  folderSession("b"),
  {
    providerId: "opencode",
    sessionId: "db-row",
    projectDir: "d:/work/target",
    sourcePath: "d:/work/storage.db",
    locator: { kind: "database", path: "d:/work/storage.db", recordId: "db-row" },
  },
  folderSession("other", "d:/work/elsewhere"),
];

const noopOptions = {
  onSessionDeleted: vi.fn(),
  onSessionArchived: vi.fn(),
  onSessionRestored: vi.fn(),
  onFolderOperationComplete: vi.fn(),
};

describe("useSessionMutations folder operations", () => {
  let queryClient: QueryClient;

  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    mocks.invoke.mockReset();
    dialogMocks.confirm.mockReset();
    for (const fn of Object.values(noopOptions)) fn.mockReset();
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
  });

  const renderMutationsHook = () =>
    renderHook(
      ({ scope }) => useSessionMutations(scope, sessions, [], new Map(), noopOptions),
      { wrapper, initialProps: { scope: "active" as const } },
    );

  describe("getFolderOperationItems", () => {
    it("collects exactly the file-backed sessions under the folder", () => {
      const { result } = renderMutationsHook();

      const { items, skippedCount } = result.current.getFolderOperationItems("d:/work/target");

      expect(items.map((item) => item.sessionId).sort()).toEqual(["a", "b"]);
      expect(skippedCount).toBe(1);
    });

    it("returns an empty batch for the All and Unknown pseudo-folders", () => {
      const { result } = renderMutationsHook();

      expect(result.current.getFolderOperationItems("all")).toEqual({
        items: [],
        skippedCount: 0,
      });
      expect(result.current.getFolderOperationItems("Unknown")).toEqual({
        items: [],
        skippedCount: 0,
      });
    });
  });

  describe("executeFolderOperation", () => {
    it("archives a folder with exactly one batch command and no per-session loop", async () => {
      mocks.invoke.mockResolvedValue([]);
      const { result } = renderMutationsHook();

      const { items } = result.current.getFolderOperationItems("d:/work/target");
      act(() => {
        result.current.executeFolderOperation("archive", items);
      });
      await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(1));

      const [command, payload] = mocks.invoke.mock.calls[0];
      expect(command).toBe("archive_sessions");
      expect(payload.items).toHaveLength(2);
      expect(payload.items.map((item: { sessionId: string }) => item.sessionId).sort()).toEqual([
        "a",
        "b",
      ]);
      // Architecture check: the folder flow must never fall back to the
      // single-session command.
      const commands = mocks.invoke.mock.calls.map(([cmd]) => cmd);
      expect(commands).not.toContain("archive_session");
    });

    it("restores a folder with exactly one restore_sessions batch command", async () => {
      mocks.invoke.mockResolvedValue([]);
      const { result } = renderMutationsHook();

      const { items } = result.current.getFolderOperationItems("d:/work/target");
      act(() => {
        result.current.executeFolderOperation("restore", items);
      });
      await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(1));

      expect(mocks.invoke.mock.calls[0][0]).toBe("restore_sessions");
    });

    it("is a no-op for an empty batch", () => {
      const { result } = renderMutationsHook();

      act(() => {
        result.current.executeFolderOperation("archive", []);
      });

      expect(mocks.invoke).not.toHaveBeenCalled();
    });
  });
});

describe("useSessionMutations destination conflicts", () => {
  let queryClient: QueryClient;
  let alertMock: ReturnType<typeof vi.spyOn>;

  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    mocks.invoke.mockReset();
    dialogMocks.confirm.mockReset();
    for (const fn of Object.values(noopOptions)) fn.mockReset();
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    alertMock = vi.spyOn(window, "alert").mockImplementation(() => {});
  });

  afterEach(() => {
    alertMock.mockRestore();
  });

  const renderMutationsHook = () =>
    renderHook(
      ({ scope }) => useSessionMutations(scope, sessions, [], new Map(), noopOptions),
      { wrapper, initialProps: { scope: "active" as const } },
    );

  // The backend refuses the move with the sentinel error unless the invoke
  // payload carries overwrite: true (the user confirmed replacement).
  // Tauri invoke rejects with the RAW STRING from the Rust command, not an
  // Error instance — the mock mirrors that runtime shape.
  const mockConflictUntilOverwrite = () => {
    mocks.invoke.mockImplementation(async (_command: string, payload?: { overwrite?: boolean }) => {
      if (!payload?.overwrite) {
        throw "Destination conflict: session already exists at d:/archived/a.jsonl";
      }
      return true;
    });
  };

  it("archive: retries with overwrite after the user confirms replacement", async () => {
    mockConflictUntilOverwrite();
    dialogMocks.confirm.mockResolvedValue(true);
    const { result } = renderMutationsHook();

    act(() => {
      result.current.handleArchive(sessions[0]);
    });
    // First invoke fails with the sentinel, the confirm resolves, the retry
    // follows — assert the final call sequence, not an intermediate count.
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(2));

    const [, firstPayload] = mocks.invoke.mock.calls[0];
    expect(firstPayload.overwrite).toBeUndefined();

    expect(dialogMocks.confirm).toHaveBeenCalledTimes(1);
    expect(dialogMocks.confirm).toHaveBeenCalledWith(
      "A session already exists at the destination. Replace it?",
      { title: "Destination conflict", kind: "warning" },
    );

    const [, retryPayload] = mocks.invoke.mock.calls[1];
    expect(retryPayload.overwrite).toBe(true);
    await waitFor(() => expect(noopOptions.onSessionArchived).toHaveBeenCalled());
    expect(alertMock).not.toHaveBeenCalled();
  });

  it("archive: cancelling the confirm leaves the session in place, silently", async () => {
    mockConflictUntilOverwrite();
    dialogMocks.confirm.mockResolvedValue(false);
    const { result } = renderMutationsHook();

    act(() => {
      result.current.handleArchive(sessions[0]);
    });
    await waitFor(() => expect(dialogMocks.confirm).toHaveBeenCalledTimes(1));

    // Flush the cancel branch before asserting nothing else happened.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(noopOptions.onSessionArchived).not.toHaveBeenCalled();
    expect(alertMock).not.toHaveBeenCalled();
  });

  it("restore: retries with overwrite after the user confirms replacement", async () => {
    mockConflictUntilOverwrite();
    dialogMocks.confirm.mockResolvedValue(true);
    const { result } = renderMutationsHook();

    act(() => {
      result.current.handleRestore(sessions[0]);
    });
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(2));

    const [firstCommand, firstPayload] = mocks.invoke.mock.calls[0];
    expect(firstCommand).toBe("restore_session");
    expect(firstPayload.overwrite).toBeUndefined();

    const [retryCommand, retryPayload] = mocks.invoke.mock.calls[1];
    expect(retryCommand).toBe("restore_session");
    expect(retryPayload.overwrite).toBe(true);
    await waitFor(() => expect(noopOptions.onSessionRestored).toHaveBeenCalled());
    expect(alertMock).not.toHaveBeenCalled();
  });

  it("non-conflicting errors still alert and never trigger the confirm", async () => {
    mocks.invoke.mockRejectedValue(new Error("Failed to move session file: access denied"));
    const { result } = renderMutationsHook();

    act(() => {
      result.current.handleArchive(sessions[0]);
    });
    await waitFor(() => expect(alertMock).toHaveBeenCalledWith("Failed to move session file: access denied"));

    expect(dialogMocks.confirm).not.toHaveBeenCalled();
    expect(mocks.invoke).toHaveBeenCalledTimes(1);
    expect(noopOptions.onSessionArchived).not.toHaveBeenCalled();
  });
});
