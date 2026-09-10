import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionMeta } from "@/types";
import { useQaExport } from "./useQaExport";

const mocks = vi.hoisted(() => ({
  save: vi.fn(),
  confirm: vi.fn(),
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  save: mocks.save,
  confirm: mocks.confirm,
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
    mocks.confirm.mockReset();
    mocks.confirm.mockResolvedValue(true);
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

  it("prefilters remote sessions out of the export request", async () => {
    mocks.save.mockResolvedValue("/tmp/out.json");
    mocks.invoke.mockResolvedValue({ count: 1, skipped: [], destPath: "/tmp/out.json" });

    const local: SessionMeta = {
      providerId: "claude",
      sessionId: "s1",
      sourcePath: "/tmp/s1.jsonl",
      locator: { kind: "file", path: "/tmp/s1.jsonl" },
    };
    const remote: SessionMeta = {
      providerId: "claude",
      sessionId: "r1",
      locator: { kind: "remote", sourceId: "ali-server", path: "/home/u/r1.jsonl" },
    };

    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(RANGE, [local, remote]);
    });

    // Pin (P0b B3): remote sessions never enter the request; the local
    // sibling still exports.
    expect(mocks.invoke).toHaveBeenCalledWith("export_qa_sessions", {
      options: expect.objectContaining({ sessions: [local] }),
    });
    expect(result.current.status.state).toBe("done");
  });

  it("errors without invoking when every visible session is remote", async () => {
    const remote: SessionMeta = {
      providerId: "claude",
      sessionId: "r1",
      locator: { kind: "remote", sourceId: "ali-server", path: "/home/u/r1.jsonl" },
    };

    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(RANGE, [remote]);
    });

    expect(mocks.save).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(result.current.status.state).toBe("error");
    expect(result.current.status.message).toContain("read-only");
  });

  it("errors when no concrete time range is selected", async () => {
    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(null, SESSIONS);
    });

    expect(result.current.status.state).toBe("error");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });

  const manySessions = (n: number): SessionMeta[] =>
    Array.from({ length: n }, (_, i) => ({
      providerId: "claude",
      sessionId: `s${i}`,
      sourcePath: `/tmp/s${i}.jsonl`,
      locator: { kind: "file" as const, path: `/tmp/s${i}.jsonl` },
    }));

  it("confirms before the save dialog when exporting more than 50 sessions", async () => {
    mocks.save.mockResolvedValue("/tmp/out.json");
    mocks.invoke.mockResolvedValue({ count: 51, skipped: [], destPath: "/tmp/out.json" });

    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(RANGE, manySessions(51));
    });

    expect(mocks.confirm).toHaveBeenCalledTimes(1);
    expect(mocks.save).toHaveBeenCalled();
    expect(mocks.invoke).toHaveBeenCalled();
  });

  it("aborts silently when the large-export confirmation is cancelled", async () => {
    mocks.confirm.mockResolvedValue(false);

    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(RANGE, manySessions(51));
    });

    expect(mocks.confirm).toHaveBeenCalledTimes(1);
    expect(mocks.save).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalled();
    expect(result.current.status.state).toBe("idle");
  });

  it("does not confirm for small exports", async () => {
    mocks.save.mockResolvedValue(null);

    const { result } = renderHook(() => useQaExport("active"));
    await act(async () => {
      await result.current.exportRange(RANGE, SESSIONS);
    });

    expect(mocks.confirm).not.toHaveBeenCalled();
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
