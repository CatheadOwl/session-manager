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
});
