import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DeleteSessionOptions } from "@/lib/api/sessions";
import type { SessionMeta } from "@/types";
import { queryKeys } from "./keys";
import { useDeleteSessionMutation, useDeleteSessionsMutation } from "./mutations";

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
