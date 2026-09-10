/**
 * Pure helpers for the Q&A export time filter.
 *
 * Presets cover the high-frequency paths; a custom range carries explicit
 * epoch-millisecond bounds (the app-wide timestamp unit — `SessionMeta`
 * timestamps are milliseconds). All computation is local-time based (the app
 * is a local desktop tool; users think in their own timezone).
 */

export type TimeRangePreset = "all" | "today" | "7d" | "30d" | "custom";

export interface TimeRange {
  preset: TimeRangePreset;
  /** Only meaningful when preset === "custom". */
  customFrom?: number;
  customTo?: number;
}

export interface ResolvedRange {
  from: number;
  to: number;
}

const DAY_MS = 86_400_000;
/** Local midnight of "now" as epoch milliseconds. */
const localMidnight = (now: Date): number =>
  new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();

/** Resolve a preset/custom range into concrete epoch-ms bounds, or
 *  `null` for "all" (no filtering). */
export const resolveTimeRange = (range: TimeRange, now = new Date()): ResolvedRange | null => {
  switch (range.preset) {
    case "all":
      return null;
    case "today":
      return { from: localMidnight(now), to: now.getTime() };
    case "7d": {
      const to = now.getTime();
      return { from: to - 7 * DAY_MS, to };
    }
    case "30d": {
      const to = now.getTime();
      return { from: to - 30 * DAY_MS, to };
    }
    case "custom": {
      const { customFrom, customTo } = range;
      if (customFrom === undefined || customTo === undefined) return null;
      // Allow either ordering by normalizing.
      return { from: Math.min(customFrom, customTo), to: Math.max(customFrom, customTo) };
    }
  }
};

/** Resolve a range for the EXPORT path: unlike `resolveTimeRange`, "all"
 *  maps to a concrete full-history window (epoch 0 → now) instead of null.
 *  The interactive export passes an explicit session list (ADR 0003), so
 *  "all" carries no size hazard beyond the visible list itself — which the
 *  >50 confirmation in useQaExport guards. Only an incomplete custom range
 *  still resolves to null (export must stay disabled then). */
export const resolveExportRange = (range: TimeRange, now = new Date()): ResolvedRange | null => {
  if (range.preset === "all") {
    return { from: 0, to: now.getTime() };
  }
  return resolveTimeRange(range, now);
};

/** Local calendar-day start of a Date (for day-picker value mapping). */
export const dateToLocalDayStart = (d: Date): Date =>
  new Date(d.getFullYear(), d.getMonth(), d.getDate());

/** Epoch milliseconds at local midnight for a day-picker Date. */
export const dayStartToEpochMs = (d: Date): number => dateToLocalDayStart(d).getTime();

/** Inclusive session-level range test (mirrors the Rust core rule). */
export const sessionWithinRange = (
  session: { lastActiveAt?: number; createdAt?: number },
  range: ResolvedRange,
): boolean => {
  const ts = session.lastActiveAt ?? session.createdAt;
  return ts !== undefined && ts >= range.from && ts <= range.to;
};
