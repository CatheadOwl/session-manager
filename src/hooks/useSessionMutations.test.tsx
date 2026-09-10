import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionMeta } from "@/types";
import { useSessionMutations } from "./useSessionMutations";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
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
