import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { renderHook, act } from "@testing-library/react";

const mocks = vi.hoisted(() => ({ copyText: vi.fn() }));
vi.mock("@/utils/clipboard", () => ({ copyText: mocks.copyText }));

import { useCopyFeedback } from "./useCopyFeedback";

describe("useCopyFeedback", () => {
  beforeEach(() => {
    mocks.copyText.mockReset();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("starts idle and echoes the idle label", () => {
    const { result } = renderHook(() => useCopyFeedback());
    expect(result.current.status).toBe("idle");
    expect(result.current.labelFor("Copy")).toBe("Copy");
  });

  it("reports `copied` on success with a confirmation label", async () => {
    mocks.copyText.mockResolvedValue(undefined);
    const { result } = renderHook(() => useCopyFeedback());

    await act(async () => {
      await result.current.copyWithFeedback("text");
    });

    expect(mocks.copyText).toHaveBeenCalledWith("text");
    expect(result.current.status).toBe("copied");
    expect(result.current.labelFor("Copy")).toBe("✓ Copied");
  });

  it("reports `failed` when the copy rejects", async () => {
    mocks.copyText.mockRejectedValue(new Error("denied"));
    const { result } = renderHook(() => useCopyFeedback());

    await act(async () => {
      await result.current.copyWithFeedback("text");
    });

    expect(result.current.status).toBe("failed");
    expect(result.current.labelFor("Copy")).toBe("Copy failed");
  });

  it("auto-resets to idle after the configured delay", async () => {
    mocks.copyText.mockResolvedValue(undefined);
    const { result } = renderHook(() => useCopyFeedback({ resetAfterMs: 500 }));

    await act(async () => {
      await result.current.copyWithFeedback("text");
    });
    expect(result.current.status).toBe("copied");

    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(result.current.status).toBe("idle");
  });

  it("does not reset before the delay elapses", async () => {
    mocks.copyText.mockResolvedValue(undefined);
    const { result } = renderHook(() => useCopyFeedback({ resetAfterMs: 1000 }));

    await act(async () => {
      await result.current.copyWithFeedback("text");
    });

    act(() => {
      vi.advanceTimersByTime(999);
    });
    expect(result.current.status).toBe("copied");
  });
});
