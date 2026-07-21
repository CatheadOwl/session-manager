import { describe, it, expect } from "vitest";
import { getProviderDisplay } from "./provider-display";

describe("getProviderDisplay", () => {
  it("maps known providers to display labels", () => {
    expect(getProviderDisplay("claude")).toEqual({ label: "Claude Code", shortLabel: "Claude" });
    expect(getProviderDisplay("gemini")).toEqual({ label: "Gemini CLI", shortLabel: "Gemini" });
    expect(getProviderDisplay("qoder")).toEqual({ label: "Qoder", shortLabel: "Qoder" });
  });

  it("echoes the id for unknown providers", () => {
    expect(getProviderDisplay("newprovider")).toEqual({
      label: "newprovider",
      shortLabel: "newprovider",
    });
  });
});
