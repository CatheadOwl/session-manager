import { describe, expect, it } from "vitest";
import {
  dayStartToEpochMs,
  resolveExportRange,
  resolveTimeRange,
  sessionWithinRange,
} from "./time-range";

const NOW = new Date(2026, 8, 9, 12, 0, 0); // 2026-09-09 12:00 local

describe("resolveTimeRange", () => {
  it("returns null for all-time preset", () => {
    expect(resolveTimeRange({ preset: "all" }, NOW)).toBeNull();
  });

  it("today starts at local midnight", () => {
    const range = resolveTimeRange({ preset: "today" }, NOW)!;
    expect(range.from).toBe(new Date(2026, 8, 9).getTime());
    expect(range.to).toBe(NOW.getTime());
  });

  it("7d/30d are trailing windows ending now (epoch ms)", () => {
    const to = NOW.getTime();
    expect(resolveTimeRange({ preset: "7d" }, NOW)).toEqual({ from: to - 7 * 86_400_000, to });
    expect(resolveTimeRange({ preset: "30d" }, NOW)).toEqual({ from: to - 30 * 86_400_000, to });
  });

  it("custom normalizes reversed bounds and requires both ends", () => {
    expect(resolveTimeRange({ preset: "custom", customFrom: 200_000, customTo: 100_000 }, NOW)).toEqual({
      from: 100_000,
      to: 200_000,
    });
    expect(resolveTimeRange({ preset: "custom", customFrom: 100_000 }, NOW)).toBeNull();
  });
});

describe("resolveExportRange", () => {
  it("maps all-time to a full-history window (epoch 0 → now)", () => {
    expect(resolveExportRange({ preset: "all" }, NOW)).toEqual({
      from: 0,
      to: NOW.getTime(),
    });
  });

  it("delegates to resolveTimeRange for concrete presets", () => {
    expect(resolveExportRange({ preset: "today" }, NOW)).toEqual(
      resolveTimeRange({ preset: "today" }, NOW),
    );
  });

  it("still returns null for an incomplete custom range", () => {
    expect(resolveExportRange({ preset: "custom", customFrom: 100_000 }, NOW)).toBeNull();
  });
});

describe("sessionWithinRange", () => {
  it("is inclusive on both ends", () => {
    const range = { from: 100, to: 200 };
    expect(sessionWithinRange({ lastActiveAt: 100 }, range)).toBe(true);
    expect(sessionWithinRange({ lastActiveAt: 200 }, range)).toBe(true);
    expect(sessionWithinRange({ lastActiveAt: 99 }, range)).toBe(false);
    expect(sessionWithinRange({ lastActiveAt: 201 }, range)).toBe(false);
  });

  it("falls back to createdAt and excludes missing timestamps", () => {
    const range = { from: 100, to: 200 };
    expect(sessionWithinRange({ createdAt: 150 }, range)).toBe(true);
    expect(sessionWithinRange({}, range)).toBe(false);
  });
});

describe("dayStartToEpochMs", () => {
  it("snaps to local day start", () => {
    const evening = new Date(2026, 8, 9, 23, 30);
    expect(dayStartToEpochMs(evening)).toBe(new Date(2026, 8, 9).getTime());
  });
});
