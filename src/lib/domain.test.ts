import { describe, it, expect } from "vitest";
import {
  deriveFolderList,
  getLifecycleOperationOptions,
  getMetadataKey,
  getSessionKey,
  normalizePinnedFolders,
  supportsLifecycleOperations,
} from "./domain";
import type { SessionMeta } from "@/types";

const session = (over: Partial<SessionMeta>): SessionMeta => ({
  providerId: "claude",
  sessionId: "s1",
  ...over,
});

describe("normalizePinnedFolders", () => {
  it("migrates pre-separator-unification backslash pins to canonical forward-slash names", () => {
    expect(normalizePinnedFolders(["D:\\Document\\Projects\\agent-dev"])).toEqual([
      "d:/Document/Projects/agent-dev",
    ]);
  });

  it("leaves already-canonical pins untouched", () => {
    expect(normalizePinnedFolders(["d:/Document/Projects/agent-dev"])).toEqual([
      "d:/Document/Projects/agent-dev",
    ]);
  });

  it("dedupes pins that collide after normalization", () => {
    expect(
      normalizePinnedFolders([
        "D:\\Document\\Projects\\agent-dev",
        "d:/Document/Projects/agent-dev",
      ]),
    ).toEqual(["d:/Document/Projects/agent-dev"]);
  });

  it("dedupes across drive case, separator style, and exact duplicates", () => {
    expect(normalizePinnedFolders(["d:/x", "D:\\x", "d:/x"])).toEqual(["d:/x"]);
  });

  it("normalizes empty stored pins to the Unknown sentinel", () => {
    expect(normalizePinnedFolders([""])).toEqual(["Unknown"]);
  });
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

  it("accepts legacy snake_case database record ids", () => {
    expect(
      getSessionKey(
        session({
          locator: { kind: "database", path: "/data/opencode.db", record_id: "row-a" },
        }),
      ),
    ).toBe("claude:s1:database:/data/opencode.db:row-a");
  });

  it("falls back to sessionId when a database locator is malformed", () => {
    expect(
      getSessionKey(
        session({
          sessionId: "row-a",
          locator: { kind: "database", path: "/data/opencode.db" },
        }),
      ),
    ).toBe("claude:row-a:database:/data/opencode.db:row-a");
  });

  it("carries sourceId in remote keys — same remote path under two sources must not collide", () => {
    const base = { kind: "remote", path: "/home/admin/.claude/projects/a/uuid.jsonl" } as const;
    const left = getSessionKey(session({ locator: { ...base, sourceId: "ali-server" } }));
    const right = getSessionKey(session({ locator: { ...base, sourceId: "other-host" } }));

    expect(left).toBe("claude:s1:remote:ali-server:/home/admin/.claude/projects/a/uuid.jsonl");
    expect(left).not.toBe(right);
    // Must not collide with a local file key for a same-shaped path either.
    expect(getSessionKey(session({ sourcePath: base.path }))).not.toBe(left);
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

  it("rejects remote-backed sessions — read-only per ADR 0007", () => {
    const meta = session({
      sourcePath: "/home/admin/.claude/projects/a/uuid.jsonl",
      locator: {
        kind: "remote",
        sourceId: "ali-server",
        path: "/home/admin/.claude/projects/a/uuid.jsonl",
      },
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
      { name: "c:/proj", count: 2, lastActiveAt: 0 },
      { name: "d:/other", count: 1, lastActiveAt: 0 },
    ]);
  });

  it("tracks the most recent lastActiveAt per folder, falling back to createdAt", () => {
    const sessions = [
      session({ sessionId: "a", projectDir: "C:\\proj", lastActiveAt: 100, createdAt: 10 }),
      session({ sessionId: "b", projectDir: "C:\\proj", lastActiveAt: 300, createdAt: 20 }),
      session({ sessionId: "c", projectDir: "D:\\other", createdAt: 50 }),
    ];

    expect(deriveFolderList(sessions)).toEqual([
      { name: "c:/proj", count: 2, lastActiveAt: 300 },
      { name: "d:/other", count: 1, lastActiveAt: 50 },
    ]);
  });

  it("merges sessions whose project dir differs only by separator style", () => {
    const sessions = [
      session({ sessionId: "a", projectDir: "D:/Document/Projects/agent-dev" }),
      session({ sessionId: "b", projectDir: "D:\\Document\\Projects\\agent-dev" }),
    ];

    expect(deriveFolderList(sessions)).toEqual([
      { name: "d:/Document/Projects/agent-dev", count: 2, lastActiveAt: 0 },
    ]);
  });

  it("maps missing project dirs to 'Unknown'", () => {
    const sessions = [session({ sessionId: "a", projectDir: null })];
    expect(deriveFolderList(sessions)).toEqual([{ name: "Unknown", count: 1, lastActiveAt: 0 }]);
  });

  it("returns an empty list for no sessions", () => {
    expect(deriveFolderList([])).toEqual([]);
  });
});
