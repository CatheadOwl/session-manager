import { describe, expect, it } from "vitest";
import { queryKeys } from "./keys";

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
