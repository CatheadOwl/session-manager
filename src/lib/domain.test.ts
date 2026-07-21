import { describe, it, expect } from "vitest";
import { getSessionKey, getMetadataKey, deriveFolderList } from "./domain";
import type { SessionMeta } from "@/types";

const session = (over: Partial<SessionMeta>): SessionMeta => ({
  providerId: "claude",
  sessionId: "s1",
  ...over,
});

describe("getSessionKey", () => {
  it("includes sourcePath for file-level uniqueness", () => {
    expect(getSessionKey(session({ sourcePath: "/data/a.jsonl" }))).toBe(
      "claude:s1:/data/a.jsonl",
    );
  });

  it("falls back to empty sourcePath segment when absent", () => {
    expect(getSessionKey(session({}))).toBe("claude:s1:");
  });
});

describe("getMetadataKey", () => {
  it("omits sourcePath so star/pin state is shared across forks", () => {
    expect(getMetadataKey(session({ sourcePath: "/data/a.jsonl" }))).toBe("claude:s1");
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
