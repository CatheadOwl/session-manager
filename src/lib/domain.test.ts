import { describe, it, expect } from "vitest";
import {
  deriveFolderList,
  getLifecycleOperationOptions,
  getMetadataKey,
  getSessionKey,
  supportsLifecycleOperations,
} from "./domain";
import type { SessionMeta } from "@/types";

const session = (over: Partial<SessionMeta>): SessionMeta => ({
  providerId: "claude",
  sessionId: "s1",
  ...over,
});

describe("getSessionKey", () => {
  it("includes sourcePath for file-level uniqueness", () => {
    expect(getSessionKey(session({ sourcePath: "/data/a.jsonl" }))).toBe(
      "claude:s1:file:/data/a.jsonl",
    );
  });

  it("falls back to empty sourcePath segment when absent", () => {
    expect(getSessionKey(session({}))).toBe("claude:s1:file:");
  });

  it("distinguishes database locators with the same path", () => {
    expect(
      getSessionKey(
        session({
          locator: { kind: "database", path: "/data/opencode.db", recordId: "row-a" },
        }),
      ),
    ).toBe("claude:s1:database:/data/opencode.db:row-a");
  });
});

describe("getMetadataKey", () => {
  it("omits sourcePath so star/pin state is shared across forks", () => {
    expect(getMetadataKey(session({ sourcePath: "/data/a.jsonl" }))).toBe("claude:s1");
  });
});

describe("lifecycle operation support", () => {
  it("builds file locator operation options from the locator path", () => {
    const meta = session({
      sourcePath: "/stale/path.jsonl",
      locator: { kind: "file", path: "/data/session.jsonl" },
    });

    expect(getLifecycleOperationOptions(meta)).toEqual({
      providerId: "claude",
      sessionId: "s1",
      sourcePath: "/data/session.jsonl",
      locator: { kind: "file", path: "/data/session.jsonl" },
    });
    expect(supportsLifecycleOperations(meta)).toBe(true);
  });

  it("falls back to legacy sourcePath for file-backed sessions", () => {
    expect(getLifecycleOperationOptions(session({ sourcePath: "/data/session.jsonl" }))).toEqual({
      providerId: "claude",
      sessionId: "s1",
      sourcePath: "/data/session.jsonl",
      locator: undefined,
    });
  });

  it("rejects database-backed sessions even when they expose a sourcePath", () => {
    const meta = session({
      sourcePath: "/data/opencode.db",
      locator: { kind: "database", path: "/data/opencode.db", recordId: "row-a" },
    });

    expect(getLifecycleOperationOptions(meta)).toBeUndefined();
    expect(supportsLifecycleOperations(meta)).toBe(false);
  });
});

describe("deriveFolderList", () => {
  it("groups by normalized project dir, counts, and sorts by name", () => {
    const sessions = [
      session({ sessionId: "a", projectDir: "C:\\proj" }),
      session({ sessionId: "b", projectDir: "C:\\proj" }),
      session({ sessionId: "c", projectDir: "D:\\other" }),
    ];

    expect(deriveFolderList(sessions)).toEqual([
      { name: "c:\\proj", count: 2 },
      { name: "d:\\other", count: 1 },
    ]);
  });

  it("maps missing project dirs to 'Unknown'", () => {
    const sessions = [session({ sessionId: "a", projectDir: null })];
    expect(deriveFolderList(sessions)).toEqual([{ name: "Unknown", count: 1 }]);
  });

  it("returns an empty list for no sessions", () => {
    expect(deriveFolderList([])).toEqual([]);
  });
});
