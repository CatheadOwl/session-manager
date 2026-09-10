import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DeleteSessionOptions } from "@/lib/api/sessions";
import type { SessionMeta } from "@/types";
import { queryKeys } from "./keys";
import { useArchiveSessionMutation, useArchiveSessionsMutation, useDeleteSessionMutation, useDeleteSessionsMutation, useRestoreSessionMutation } from "./mutations";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

describe("delete session mutations", () => {
  let queryClient: QueryClient;

  const archivedSession: SessionMeta = {
    providerId: "claude",
    sessionId: "archived-session",
    sourcePath: "/data/archive/session.jsonl",
    locator: { kind: "file", path: "/data/archive/session.jsonl" },
  };

  const deleteOptions: DeleteSessionOptions = {
    providerId: archivedSession.providerId,
    sessionId: archivedSession.sessionId,
    sourcePath: archivedSession.sourcePath ?? "",
    locator: archivedSession.locator,
  };

  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    mocks.invoke.mockReset();
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });
  });

  it("removes a single deleted session from the archived cache when archived is the source scope", async () => {
    mocks.invoke.mockResolvedValue(true);
    queryClient.setQueryData(queryKeys.sessions("active"), [archivedSession]);
    queryClient.setQueryData(queryKeys.sessions("archived"), [archivedSession]);

    const { result } = renderHook(
      () => useDeleteSessionMutation("archived"),
      { wrapper },
    );

    await result.current.mutateAsync(deleteOptions);

    expect(queryClient.getQueryData(queryKeys.sessions("archived"))).toEqual([]);
    expect(queryClient.getQueryData(queryKeys.sessions("active"))).toEqual([archivedSession]);
  });

  it("removes batch-deleted sessions from the archived cache when archived is the source scope", async () => {
    mocks.invoke.mockResolvedValue([{ ...deleteOptions, success: true }]);
    queryClient.setQueryData(queryKeys.sessions("active"), [archivedSession]);
    queryClient.setQueryData(queryKeys.sessions("archived"), [archivedSession]);

    const { result } = renderHook(
      () => useDeleteSessionsMutation("archived"),
      { wrapper },
    );

    await result.current.mutateAsync([deleteOptions]);

    expect(queryClient.getQueryData(queryKeys.sessions("archived"))).toEqual([]);
    expect(queryClient.getQueryData(queryKeys.sessions("active"))).toEqual([archivedSession]);
  });
});

describe("archive/restore mutation cache semantics", () => {
  let queryClient: QueryClient;

  const activeSession: SessionMeta = {
    providerId: "claude",
    sessionId: "active-session",
    sourcePath: "/data/projects/session.jsonl",
    locator: { kind: "file", path: "/data/projects/session.jsonl" },
  };

  const archiveOptions: DeleteSessionOptions = {
    providerId: activeSession.providerId,
    sessionId: activeSession.sessionId,
    sourcePath: activeSession.sourcePath ?? "",
    locator: activeSession.locator,
  };

  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  const seedCaches = () => {
    queryClient.setQueryData(queryKeys.sessions("active"), [activeSession]);
    queryClient.setQueryData(queryKeys.sessions("archived"), []);
  };

  beforeEach(() => {
    mocks.invoke.mockReset();
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });
  });

  it("archive removes the session from the active cache and invalidates both scopes", async () => {
    mocks.invoke.mockResolvedValue(true);
    seedCaches();
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useArchiveSessionMutation(), { wrapper });

    await result.current.mutateAsync(archiveOptions);

    expect(queryClient.getQueryData(queryKeys.sessions("active"))).toEqual([]);
    const invalidated = invalidateSpy.mock.calls.map(([options]) => options?.queryKey);
    expect(invalidated).toContainEqual(queryKeys.sessions("active"));
    expect(invalidated).toContainEqual(queryKeys.sessions("archived"));
  });

  it("batch archive removes only successful outcomes from the active cache and invalidates both scopes", async () => {
    const failedSession: SessionMeta = {
      ...activeSession,
      sessionId: "failed-session",
      sourcePath: "/data/projects/failed.jsonl",
    };
    const failedOptions: DeleteSessionOptions = {
      ...archiveOptions,
      sessionId: failedSession.sessionId,
      sourcePath: failedSession.sourcePath ?? "",
    };
    mocks.invoke.mockResolvedValue([
      { ...archiveOptions, success: true, error: null },
      { ...failedOptions, success: false, error: "session source not found" },
    ]);
    queryClient.setQueryData(queryKeys.sessions("active"), [activeSession, failedSession]);
    queryClient.setQueryData(queryKeys.sessions("archived"), []);
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useArchiveSessionsMutation(), { wrapper });

    await result.current.mutateAsync([archiveOptions, failedOptions]);

    // Successful item is removed optimistically; the failed item stays in the
    // list until the invalidation refetch restores the truth.
    expect(queryClient.getQueryData(queryKeys.sessions("active"))).toEqual([failedSession]);
    const invalidated = invalidateSpy.mock.calls.map(([options]) => options?.queryKey);
    expect(invalidated).toContainEqual(queryKeys.sessions("active"));
    expect(invalidated).toContainEqual(queryKeys.sessions("archived"));
  });

  it("restore removes the session from the archived cache and invalidates both scopes", async () => {
    mocks.invoke.mockResolvedValue(true);
    queryClient.setQueryData(queryKeys.sessions("active"), []);
    queryClient.setQueryData(queryKeys.sessions("archived"), [activeSession]);
    const invalidateSpy = vi.spyOn(queryClient, "invalidateQueries");

    const { result } = renderHook(() => useRestoreSessionMutation(), { wrapper });

    await result.current.mutateAsync(archiveOptions);

    expect(queryClient.getQueryData(queryKeys.sessions("archived"))).toEqual([]);
    const invalidated = invalidateSpy.mock.calls.map(([options]) => options?.queryKey);
    expect(invalidated).toContainEqual(queryKeys.sessions("active"));
    expect(invalidated).toContainEqual(queryKeys.sessions("archived"));
  });
});
