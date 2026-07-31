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

export const sessionLocatorKey = (locator: SessionLocator | undefined, sourcePath?: string) => {
  if (locator?.kind === "database") {
    return ["database", locator.path, locator.recordId] as const;
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
  ) =>
    [
      "sessionDetail",
      providerId,
      ...(typeof sourcePathOrLocator === "string"
        ? sessionLocatorKey(undefined, sourcePathOrLocator)
        : sessionLocatorKey(sourcePathOrLocator)),
    ] as const,

  appMetadata: () => ["appMetadata"] as const,

  forkTree: (scope: "active" | "archived", projectDir?: string) =>
    ["forkTree", scope, projectDir ?? "__all__"] as const,
};
