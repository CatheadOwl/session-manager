import { describe, it, expect } from "vitest";
import {
  getBaseName,
  normalizeProjectDir,
  formatSessionTitle,
  formatTimestamp,
} from "./format";
import type { SessionMeta } from "@/types";

describe("getBaseName", () => {
  it("extracts the last segment of a windows path", () => {
    expect(getBaseName("C:\\Users\\foo\\project")).toBe("project");
  });

  it("extracts the last segment of a posix path", () => {
    expect(getBaseName("/home/user/project")).toBe("project");
  });

  it("strips trailing separators before extracting", () => {
    expect(getBaseName("/home/user/project/")).toBe("project");
    expect(getBaseName("C:\\proj\\")).toBe("proj");
  });

  it("handles mixed separators", () => {
    expect(getBaseName("C:\\foo/bar")).toBe("bar");
  });

  it("returns the input when there is no separator", () => {
    expect(getBaseName("project")).toBe("project");
  });

  it("returns empty string for empty/null/undefined", () => {
    expect(getBaseName("")).toBe("");
    expect(getBaseName(null)).toBe("");
    expect(getBaseName(undefined)).toBe("");
  });
});

describe("normalizeProjectDir", () => {
  it("lowercases the drive letter of a windows path", () => {
    expect(normalizeProjectDir("C:\\Users\\foo")).toBe("c:\\Users\\foo");
    expect(normalizeProjectDir("D:\\proj")).toBe("d:\\proj");
  });

  it("leaves posix paths untouched", () => {
    expect(normalizeProjectDir("/home/user")).toBe("/home/user");
  });

  it("returns 'Unknown' for empty/null/undefined", () => {
    expect(normalizeProjectDir("")).toBe("Unknown");
    expect(normalizeProjectDir(null)).toBe("Unknown");
    expect(normalizeProjectDir(undefined)).toBe("Unknown");
  });
});

describe("formatSessionTitle", () => {
  const base: SessionMeta = { providerId: "claude", sessionId: "abcdef1234567890" };

  it("prefers the explicit title", () => {
    expect(formatSessionTitle({ ...base, title: "My Session" })).toBe("My Session");
  });

  it("falls back to the project dir basename", () => {
    expect(formatSessionTitle({ ...base, projectDir: "C:\\work\\demo" })).toBe("demo");
  });

  it("falls back to the short session id", () => {
    expect(formatSessionTitle({ ...base })).toBe("abcdef12");
  });
});

describe("formatTimestamp", () => {
  it("returns an em dash for missing/zero values", () => {
    expect(formatTimestamp(undefined)).toBe("—");
    expect(formatTimestamp(0)).toBe("—");
  });

  it("formats a real timestamp into a non-empty numeric string", () => {
    const out = formatTimestamp(1_700_000_000_000);
    expect(out).not.toBe("—");
    expect(out).toMatch(/\d/);
  });
});
