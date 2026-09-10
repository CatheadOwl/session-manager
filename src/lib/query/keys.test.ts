import { describe, expect, it } from "vitest";
import { queryKeys } from "./keys";

describe("queryKeys invalidation prefixes", () => {
  it("sessionsAll is a prefix of every sessions(scope) key", () => {
    const prefix = queryKeys.sessionsAll();
    for (const scope of ["active", "archived"] as const) {
      expect(queryKeys.sessions(scope).slice(0, prefix.length)).toEqual(prefix);
    }
  });

  it("forkTreeAll is a prefix of every forkTree(scope, projectDir) key", () => {
    const prefix = queryKeys.forkTreeAll();
    expect(queryKeys.forkTree("active").slice(0, prefix.length)).toEqual(prefix);
    expect(
      queryKeys.forkTree("archived", "D:/proj").slice(0, prefix.length),
    ).toEqual(prefix);
  });
});

describe("queryKeys.sessionDetail", () => {
  it("keeps legacy sourcePath detail keys file-scoped", () => {
    expect(queryKeys.sessionDetail("claude", "/data/a.jsonl")).toEqual([
      "sessionDetail",
      "claude",
      "file",
      "/data/a.jsonl",
    ]);
  });

  it("distinguishes database records in the same file", () => {
    const left = queryKeys.sessionDetail("opencode", {
      kind: "database",
      path: "/data/opencode.db",
      recordId: "left",
    });
    const right = queryKeys.sessionDetail("opencode", {
      kind: "database",
      path: "/data/opencode.db",
      recordId: "right",
    });

    expect(left).not.toEqual(right);
  });

  it("accepts legacy snake_case database record ids from the IPC boundary", () => {
    const left = queryKeys.sessionDetail("opencode", {
      kind: "database",
      path: "/data/opencode.db",
      record_id: "left",
    });
    const right = queryKeys.sessionDetail("opencode", {
      kind: "database",
      path: "/data/opencode.db",
      record_id: "right",
    });

    expect(left).toEqual([
      "sessionDetail",
      "opencode",
      "database",
      "/data/opencode.db",
      "left",
    ]);
    expect(left).not.toEqual(right);
  });

  it("falls back to the session id when a database locator is malformed", () => {
    expect(
      queryKeys.sessionDetail(
        "opencode",
        {
          kind: "database",
          path: "/data/opencode.db",
        },
        "row-a",
      ),
    ).toEqual(["sessionDetail", "opencode", "database", "/data/opencode.db", "row-a"]);
  });
});
