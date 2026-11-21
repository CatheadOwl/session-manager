import { useCallback, useEffect, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { useSettingsQuery } from "@/lib/query/queries";

export type UpdateStatus = "idle" | "checking" | "available" | "downloading" | "ready" | "error";

export function useUpdater() {
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [update, setUpdate] = useState<Update | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Settings gate: `update.autoCheck === false` opts out of the
  // startup check. The gate is a read through IPC, not a local flag —
  // hand-editing settings.json + restart changes behavior. While the settings
  // query is still loading (`data === undefined`) the auto-check is HELD: an
  // opted-out user must not fire even one stray check request while the gate
  // resolves (fail closed; the default-true path simply starts one render
  // later, once the query settles).
  const { data: settings } = useSettingsQuery();
  const settingsLoaded = settings !== undefined;
  const autoCheckDisabled =
    settings?.values["update.autoCheck"]?.bool === false;

  const checkForUpdate = useCallback(async () => {
    try {
      setStatus("checking");
      setError(null);
      const u = await check();
      // Deliberately NO cancellation guard: every resolution is applied. The
      // former cancelledRef discarded a resolution when a gate flip landed
      // mid-flight (its cleanup ran on dep changes, but the early-return
      // path never reset the flag) — status stuck on "checking" forever,
      // which busy-disable then turned into an unrecoverable check row.
      // A resolution is a fact about the remote, valid regardless of the
      // flip that landed meanwhile; React 18 makes a post-unmount setState
      // a harmless no-op, and this hook's only mount point
      // (SessionManagerPage, rendered unconditionally by App) never unmounts.
      if (u) {
        setUpdate(u);
        setStatus("available");
      } else {
        setStatus("idle");
      }
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }, []);

  useEffect(() => {
    if (!settingsLoaded || autoCheckDisabled) return;
    checkForUpdate();
  }, [settingsLoaded, autoCheckDisabled, checkForUpdate]);

  const installUpdate = useCallback(async () => {
    if (!update) return;
    try {
      setStatus("downloading");
      await update.downloadAndInstall();
      setStatus("ready");
      await relaunch();
    } catch (e) {
      setError(String(e));
      setStatus("error");
    }
  }, [update]);

  // `checkForUpdate` is the dedicated manual-check entry (Settings check row):
  // it runs regardless of the auto gate and is the recovery path from a
  // failed check. Callers disable their trigger while `status` is busy
  // (checking/downloading) — the hook itself does not queue a second check.
  // Never bind retry actions to `installUpdate`: it no-ops on a null update
  // (the `error` state), which is exactly the bug class the guard exists for.
  return { status, update, error, checkForUpdate, installUpdate };
}
