import { describe, it, expect } from "vitest";
import { getRoleDisplay } from "./role-display";

describe("getRoleDisplay", () => {
  it("maps known roles case-insensitively", () => {
    expect(getRoleDisplay("assistant")).toEqual({ label: "Assistant", tone: "assistant" });
    expect(getRoleDisplay("USER")).toEqual({ label: "User", tone: "user" });
    expect(getRoleDisplay("System")).toEqual({ label: "System", tone: "system" });
    expect(getRoleDisplay("tool")).toEqual({ label: "Tool", tone: "tool" });
  });

  it("falls back to an unknown tone for unrecognized roles", () => {
    expect(getRoleDisplay("debug")).toEqual({ label: "debug", tone: "unknown" });
  });

  it("uses 'Unknown' label for an empty role", () => {
    expect(getRoleDisplay("")).toEqual({ label: "Unknown", tone: "unknown" });
  });
});
