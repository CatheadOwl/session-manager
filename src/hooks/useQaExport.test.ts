import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionMeta } from "@/types";
import { useQaExport } from "./useQaExport";

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: mocks.save,
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

const RANGE = { from: 1_000, to: 2_000 };

const SESSIONS: SessionMeta[] = [
  {
    providerId: "claude",
    sessionId: "s1",
    sourcePath: "/tmp/s1.jsonl",
    locator: { kind: "file", path: "/tmp/s1.jsonl" },
  },
];

describe("useQaExport", () => {
  beforeEach(() => {
    mocks.save.mockReset();
    mocks.invoke.mockReset();
  });

  it("aborts silently when the save dialog is cancelled", async () => {
    mocks.save.mockResolvedValue(null);

    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(RANGE, SESSIONS);
    });

    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(result.current.status.state).toBe("idle");
  });

  it("errors without invoking when there are no visible sessions", async () => {
    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(RANGE, []);
    });

    expect(mocks.save).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(result.current.status.state).toBe("error");
    expect(result.current.status.message).toContain("No visible sessions");
  });

  it("errors when no concrete time range is selected", async () => {
    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(null, SESSIONS);
    });

    expect(result.current.status.state).toBe("error");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  it("exports the visible list with overwrite confirmed by the dialog", async () => {
    mocks.save.mockResolvedValue("/tmp/out.json");
    mocks.invoke.mockResolvedValue({ count: 1, skipped: [], destPath: "/tmp/out.json" });

    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(RANGE, SESSIONS);
    });

    expect(mocks.invoke).toHaveBeenCalledWith("export_qa_sessions", {
      options: {
        scope: "active",
        from: RANGE.from,
        to: RANGE.to,
        sessions: SESSIONS,
        destPath: "/tmp/out.json",
        format: "json",
        // The native dialog already confirmed replacement; see ADR notes in useQaExport.
        overwrite: true,
      },
    });
    expect(result.current.status.state).toBe("done");
    expect(result.current.status.message).toContain("Exported 1 session");
  });

  it("infers markdown format from the destination extension", async () => {
    mocks.save.mockResolvedValue("/tmp/out.md");
    mocks.invoke.mockResolvedValue({ count: 1, skipped: [], destPath: "/tmp/out.md" });

    const { result } = renderHook(() => useQaExport("archived"));
    await act(async () => {
      await result.current.exportRange(RANGE, SESSIONS);
    });

    expect(mocks.invoke).toHaveBeenCalledWith(
      "export_qa_sessions",
      expect.objectContaining({
        options: expect.objectContaining({ format: "markdown", scope: "archived" }),
      }),
    );
  });

  it("surfaces backend errors as a persistent error status", async () => {
    mocks.save.mockResolvedValue("/tmp/out.json");
    mocks.invoke.mockRejectedValue(new Error("Destination file already exists"));

    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(RANGE, SESSIONS);
    });

    expect(result.current.status.state).toBe("error");
    expect(result.current.status.message).toContain("already exists");
  });

  it("resets status on clearStatus", async () => {
    mocks.save.mockResolvedValue(null);
    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(null, SESSIONS);
    });
    expect(result.current.status.state).toBe("error");

    act(() => {
      result.current.clearStatus();
    });
    expect(result.current.status.state).toBe("idle");
  });
});
