/**
 * Centralized React Query key factory.
 *
 * Every query key in the app should be defined here so that:
 * 1. Mutations can invalidate the correct queries without hard-coded strings.
 * 2. Cross-module cache reads (e.g. useLatestMessage → sessionDetail) use the same key.
 * 3. Stale keys are easy to audit.
 *
 * @tanstack/react-query uses array keys; each factory returns a const tuple.
 */
import type { SessionLocator } from "@/types";

type DatabaseLocatorWire = Extract<SessionLocator, { kind: "database" }> & {
  record_id?: string;
};

const databaseRecordId = (locator: DatabaseLocatorWire) =>
  locator.recordId ?? locator.record_id ?? "";

export const sessionLocatorKey = (
  locator: SessionLocator | undefined,
  sourcePath?: string,
  fallbackSessionId?: string,
) => {
  if (locator?.kind === "database") {
    return ["database", locator.path, databaseRecordId(locator) || fallbackSessionId || ""] as const;
  }

  if (locator?.kind === "file") {
    return ["file", locator.path] as const;
  }

  return ["file", sourcePath ?? ""] as const;
};

export const queryKeys = {
  sessions: (scope: "active" | "archived") => ["sessions", scope] as const,

  sessionDetail: (
    providerId: string,
    sourcePathOrLocator: string | SessionLocator | undefined,
    sessionId?: string,
  ) =>
    [
      "sessionDetail",
      providerId,
      ...(typeof sourcePathOrLocator === "string"
        ? sessionLocatorKey(undefined, sourcePathOrLocator)
        : sessionLocatorKey(sourcePathOrLocator, undefined, sessionId)),
    ] as const,

  appMetadata: () => ["appMetadata"] as const,

  forkTree: (scope: "active" | "archived", projectDir?: string) =>
    ["forkTree", scope, projectDir ?? "__all__"] as const,
};
