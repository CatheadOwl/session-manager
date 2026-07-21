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

export const queryKeys = {
  sessions: (scope: "active" | "archived") => ["sessions", scope] as const,

  sessionDetail: (providerId: string, sourcePath: string) =>
    ["sessionDetail", providerId, sourcePath] as const,

  appMetadata: () => ["appMetadata"] as const,

  forkTree: (scope: "active" | "archived", projectDir?: string) =>
    ["forkTree", scope, projectDir ?? "__all__"] as const,
};
