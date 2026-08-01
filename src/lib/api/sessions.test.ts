import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionMeta } from "@/types";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

describe("sessionsApi handle-aware IPC payloads", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue([]);
  });

  it("preserves legacy sourcePath detail calls", async () => {
    const { sessionsApi } = await import("./sessions");

    await sessionsApi.getSessionDetail("claude", "/data/session.jsonl");

    expect(mocks.invoke).toHaveBeenCalledWith("get_session_detail", {
      providerId: "claude",
      sessionId: "",
      sourcePath: "/data/session.jsonl",
    });
  });

  it("sends sessionId and locator for handle-aware detail calls", async () => {
    const { sessionHandleOptionsFromMeta, sessionsApi } = await import("./sessions");
    const session: SessionMeta = {
      providerId: "opencode",
      sessionId: "row-a",
      sourcePath: "/data/opencode.db",
      locator: { kind: "database", path: "/data/opencode.db", recordId: "row-a" },
    };

    await sessionsApi.getSessionDetail(sessionHandleOptionsFromMeta(session));

    expect(mocks.invoke).toHaveBeenCalledWith("get_session_detail", {
      providerId: "opencode",
      sessionId: "row-a",
      sourcePath: "/data/opencode.db",
      locator: { kind: "database", path: "/data/opencode.db", recordId: "row-a" },
    });
  });

  it("sends sessionId and locator for handle-aware message calls", async () => {
    const { sessionHandleOptionsFromMeta, sessionsApi } = await import("./sessions");
    const session: SessionMeta = {
      providerId: "claude",
      sessionId: "session-1",
      sourcePath: "/data/session.jsonl",
      locator: { kind: "file", path: "/data/session.jsonl" },
    };

    await sessionsApi.getMessages(sessionHandleOptionsFromMeta(session));

    expect(mocks.invoke).toHaveBeenCalledWith("get_session_messages", {
      providerId: "claude",
      sessionId: "session-1",
      sourcePath: "/data/session.jsonl",
      locator: { kind: "file", path: "/data/session.jsonl" },
    });
  });

  it("only builds handle options for loadable session metadata", async () => {
    const { loadableSessionHandleOptionsFromMeta } = await import("./sessions");

    expect(loadableSessionHandleOptionsFromMeta()).toBeUndefined();
    expect(
      loadableSessionHandleOptionsFromMeta({
        providerId: "claude",
        sessionId: "session-1",
      }),
    ).toBeUndefined();
    expect(
      loadableSessionHandleOptionsFromMeta({
        providerId: "claude",
        sessionId: "session-1",
        locator: { kind: "file", path: "/data/session.jsonl" },
      }),
    ).toEqual({
      providerId: "claude",
      sessionId: "session-1",
      sourcePath: undefined,
      locator: { kind: "file", path: "/data/session.jsonl" },
    });
  });

  it("does not treat database locators without a record id as loadable", async () => {
    const { loadableSessionHandleOptionsFromMeta } = await import("./sessions");

    expect(
      loadableSessionHandleOptionsFromMeta({
        providerId: "opencode",
        sessionId: "row-a",
        sourcePath: "/data/opencode.db",
        locator: { kind: "database", path: "/data/opencode.db" },
      }),
    ).toBeUndefined();
  });
});
