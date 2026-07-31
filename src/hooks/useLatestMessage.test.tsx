import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { queryKeys } from "@/lib/query/keys";
import type { SessionDetail, SessionMeta } from "@/types";
import { useLatestMessage } from "./useLatestMessage";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

describe("useLatestMessage", () => {
  let queryClient: QueryClient;

  beforeEach(() => {
    mocks.invoke.mockReset();
    mocks.invoke.mockResolvedValue([]);
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
      },
    });
  });

  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  it("reads cached detail using the structured locator key from session metadata", async () => {
    const session: SessionMeta = {
      providerId: "opencode",
      sessionId: "row-a",
      sourcePath: "/data/opencode.db",
      locator: { kind: "database", path: "/data/opencode.db", recordId: "row-a" },
    };
    const detail: SessionDetail = {
      messages: [
        { role: "assistant", content: "older" },
        { role: "assistant", content: "latest" },
      ],
      qaPairs: [],
    };
    queryClient.setQueryData(
      queryKeys.sessionDetail(session.providerId, session.locator),
      detail,
    );

    const { result } = renderHook(() => useLatestMessage({ session }), { wrapper });
    const latest = await result.current.getLatestMessage();

    expect(latest?.content).toBe("latest");
    expect(mocks.invoke).not.toHaveBeenCalled();
  });
});
