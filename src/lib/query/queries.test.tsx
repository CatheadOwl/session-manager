import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionDetail, SessionMeta } from "@/types";
import { useSessionDetailQuery } from "./queries";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

describe("useSessionDetailQuery", () => {
  let queryClient: QueryClient;

  beforeEach(() => {
    mocks.invoke.mockReset();
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });
  });

  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  const session = (sessionId: string, recordId = sessionId): SessionMeta => ({
    providerId: "opencode",
    sessionId,
    sourcePath: "/data/opencode.db",
    locator: { kind: "database", path: "/data/opencode.db", recordId },
  });

  it("does not show the previous session detail while a different DB session loads", async () => {
    const firstDetail: SessionDetail = {
      messages: [{ role: "user", content: "first session" }],
      qaPairs: [],
    };

    mocks.invoke
      .mockResolvedValueOnce(firstDetail)
      .mockImplementationOnce(() => new Promise(() => undefined));

    const { result, rerender } = renderHook(
      ({ selected }) => useSessionDetailQuery(selected),
      { initialProps: { selected: session("ses_a") }, wrapper },
    );

    await waitFor(() => {
      expect(result.current.data?.messages[0]?.content).toBe("first session");
    });

    rerender({ selected: session("ses_b") });

    expect(result.current.data).toBeUndefined();
    expect(result.current.isFetching).toBe(true);
  });
});
