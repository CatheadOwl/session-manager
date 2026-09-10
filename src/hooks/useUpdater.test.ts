import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor, act } from "@testing-library/react";

// Mock the Tauri plugins so the hook's logic can be exercised without a native runtime.
const mocks = vi.hoisted(() => ({
  check: vi.fn(),
  relaunch: vi.fn(),
  useSettingsQuery: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-updater", () => ({
  check: mocks.check,
}));
vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: mocks.relaunch,
}));

// The settings gate reads `update.autoCheck` through the settings query.
vi.mock("@/lib/query/queries", () => ({
  useSettingsQuery: mocks.useSettingsQuery,
}));

import { useUpdater } from "./useUpdater";

type SettingsData = { values: Record<string, { bool: boolean }> };

function mockSettings(autoCheck: boolean) {
  const data: SettingsData = { values: { "update.autoCheck": { bool: autoCheck } } };
  mocks.useSettingsQuery.mockReturnValue({ data });
  return data;
}

function makeFakeUpdate() {
  return {
    version: "0.2.0",
    downloadAndInstall: vi.fn().mockResolvedValue(undefined),
  };
}

describe("useUpdater", () => {
  beforeEach(() => {
    mocks.check.mockReset();
    mocks.relaunch.mockReset();
    mocks.relaunch.mockResolvedValue(undefined);
    // Default: autoCheck enabled (the settings default).
    mockSettings(true);
  });

  it("reports `available` and stores the update when check finds one", async () => {
    const fakeUpdate = makeFakeUpdate();
    mocks.check.mockResolvedValue(fakeUpdate);

    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.status).toBe("available"));
    expect(result.current.update).toBe(fakeUpdate);
    expect(result.current.error).toBeNull();
  });

  it("returns to `idle` when check finds no update", async () => {
    mocks.check.mockResolvedValue(null);

    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.status).toBe("idle"));
    expect(result.current.update).toBeNull();
  });

  it("reports `error` with a message when check rejects", async () => {
    mocks.check.mockRejectedValue(new Error("network down"));

    const { result } = renderHook(() => useUpdater());

    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.error).toContain("network down");
    expect(result.current.update).toBeNull();
  });

  it("downloads, reaches `ready`, and relaunches on installUpdate", async () => {
    const fakeUpdate = makeFakeUpdate();
    mocks.check.mockResolvedValue(fakeUpdate);

    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.status).toBe("available"));

    await act(async () => {
      await result.current.installUpdate();
    });

    expect(fakeUpdate.downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(mocks.relaunch).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe("ready");
  });

  // Regression: installUpdate must be a guarded no-op when no update object exists
  // (e.g. in the `error` state). Binding "Retry" to installUpdate would otherwise
  // silently do nothing — see the retryCheck test below for the real recovery path.
  it("installUpdate is a no-op when there is no resolved update", async () => {
    mocks.check.mockRejectedValue(new Error("check failed"));

    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.status).toBe("error"));

    await act(async () => {
      await result.current.installUpdate();
    });

    expect(mocks.relaunch).not.toHaveBeenCalled();
    expect(result.current.status).toBe("error");
  });

  // Regression: retryCheck re-runs the network check, providing a real recovery
  // path from `error` (distinct from installUpdate).
  it("retryCheck re-runs the check and can recover from error", async () => {
    const fakeUpdate = makeFakeUpdate();
    mocks.check
      .mockRejectedValueOnce(new Error("transient"))
      .mockResolvedValueOnce(fakeUpdate);

    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.status).toBe("error"));

    await act(async () => {
      await result.current.retryCheck();
    });

    await waitFor(() => expect(result.current.status).toBe("available"));
    expect(mocks.check).toHaveBeenCalledTimes(2);
    expect(result.current.update).toBe(fakeUpdate);
  });

  // Settings gate (ADR 0006): update.autoCheck === false skips the mount auto-check.
  it("stays idle and never calls check when update.autoCheck is false", async () => {
    mockSettings(false);

    const { result } = renderHook(() => useUpdater());

    // Give any would-be async check a chance to run.
    await act(async () => {
      await Promise.resolve();
    });

    expect(mocks.check).not.toHaveBeenCalled();
    expect(result.current.status).toBe("idle");
    expect(result.current.update).toBeNull();
    expect(result.current.error).toBeNull();
  });

  // Gate timing: while the settings query is still loading there is no
  // settings data yet — the auto-check must be HELD, not fired-and-discarded,
  // so an opted-out user never triggers even one stray check request.
  it("does not auto-check while settings are still loading", async () => {
    mocks.useSettingsQuery.mockReturnValue({ data: undefined });
    mocks.check.mockResolvedValue(makeFakeUpdate());

    renderHook(() => useUpdater());

    await act(async () => {
      await Promise.resolve();
    });

    expect(mocks.check).not.toHaveBeenCalled();
  });

  // Once the settings resolve with the default (autoCheck true), the held
  // auto-check starts.
  it("auto-checks after settings load with autoCheck true", async () => {
    const fakeUpdate = makeFakeUpdate();
    mocks.check.mockResolvedValue(fakeUpdate);
    // Start in the loading state, then resolve to enabled.
    let data: SettingsData | undefined = undefined;
    mocks.useSettingsQuery.mockImplementation(() => ({ data }));
    const { result, rerender } = renderHook(() => useUpdater());

    await act(async () => {
      await Promise.resolve();
    });
    expect(mocks.check).not.toHaveBeenCalled();

    data = { values: { "update.autoCheck": { bool: true } } };
    await act(async () => {
      rerender();
    });

    await waitFor(() => expect(result.current.status).toBe("available"));
    expect(mocks.check).toHaveBeenCalledTimes(1);
  });

  // The gate only silences automatic checks — retryCheck (user-initiated) stays ungated.
  it("retryCheck still runs the check when update.autoCheck is false", async () => {
    mockSettings(false);
    const fakeUpdate = makeFakeUpdate();
    mocks.check.mockResolvedValue(fakeUpdate);

    const { result } = renderHook(() => useUpdater());

    await act(async () => {
      await result.current.retryCheck();
    });

    expect(mocks.check).toHaveBeenCalledTimes(1);
    expect(result.current.status).toBe("available");
    expect(result.current.update).toBe(fakeUpdate);
  });

  it("surfaces download failure as `error`", async () => {    const fakeUpdate = makeFakeUpdate();
    fakeUpdate.downloadAndInstall.mockRejectedValue(new Error("bad signature"));
    mocks.check.mockResolvedValue(fakeUpdate);

    const { result } = renderHook(() => useUpdater());
    await waitFor(() => expect(result.current.status).toBe("available"));

    await act(async () => {
      await result.current.installUpdate();
    });

    expect(result.current.status).toBe("error");
    expect(result.current.error).toContain("bad signature");
    expect(mocks.relaunch).not.toHaveBeenCalled();
  });
});
