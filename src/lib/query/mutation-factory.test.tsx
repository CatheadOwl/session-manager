import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DeleteSessionOptions, DeleteSessionResult } from "@/lib/api/sessions";
import type { SessionDetail, SessionMeta } from "@/types";
import { queryKeys } from "./keys";
import { useBatchSessionMutation, useSingleSessionMutation } from "./mutation-factory";

describe("session mutation cache removal", () => {
  let queryClient: QueryClient;
  const dbPath = "/data/opencode.db";

  const dbSession: SessionMeta = {
    providerId: "opencode",
    sessionId: "ses_a",
    sourcePath: dbPath,
    locator: { kind: "database", path: dbPath, recordId: "row-a" },
  };

  const dbOptions: DeleteSessionOptions = {
    providerId: dbSession.providerId,
    sessionId: dbSession.sessionId,
    sourcePath: dbPath,
    locator: dbSession.locator,
  };

  const detail: SessionDetail = {
    messages: [{ role: "user", content: "cached DB detail" }],
    qaPairs: [],
  };

  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });
  });

  it("removes single-session DB detail cache by locator identity", async () => {
    const apiMethod = vi.fn<(_input: DeleteSessionOptions) => Promise<void>>().mockResolvedValue(undefined);
    queryClient.setQueryData(queryKeys.sessions("active"), [dbSession]);
    queryClient.setQueryData(queryKeys.sessionDetail(dbSession.providerId, dbSession.locator), detail);

    const { result } = renderHook(
      () => useSingleSessionMutation(apiMethod, "active", ["active"]),
      { wrapper },
    );

    await result.current.mutateAsync(dbOptions);

    expect(queryClient.getQueryData(queryKeys.sessionDetail(dbSession.providerId, dbSession.locator))).toBeUndefined();
  });

  it("removes batch DB detail cache by locator identity", async () => {
    const apiMethod = vi.fn<(_items: DeleteSessionOptions[]) => Promise<DeleteSessionResult[]>>()
      .mockResolvedValue([{ ...dbOptions, success: true }]);
    queryClient.setQueryData(queryKeys.sessions("active"), [dbSession]);
    queryClient.setQueryData(queryKeys.sessionDetail(dbSession.providerId, dbSession.locator), detail);

    const { result } = renderHook(
      () => useBatchSessionMutation(apiMethod, "active", ["active"]),
      { wrapper },
    );

    await result.current.mutateAsync([dbOptions]);

    expect(queryClient.getQueryData(queryKeys.sessionDetail(dbSession.providerId, dbSession.locator))).toBeUndefined();
  });
});
