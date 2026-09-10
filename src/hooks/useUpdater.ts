import { useCallback, useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { useSettingsQuery } from "@/lib/query/queries";

export type UpdateStatus = "idle" | "checking" | "available" | "downloading" | "ready" | "error";

export function useUpdater() {
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [update, setUpdate] = useState<Update | null>(null);
  const [error, setError] = useState<string | null>(null);
  const cancelledRef = useRef(false);

  // Settings gate (ADR 0006): `update.autoCheck === false` opts out of the
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
      if (cancelledRef.current) return;
      if (u) {
        setUpdate(u);
        setStatus("available");
      } else {
        setStatus("idle");
      }
    } catch (e) {
      if (!cancelledRef.current) {
        setError(String(e));
        setStatus("error");
      }
    }
  }, []);

  useEffect(() => {
    if (!settingsLoaded || autoCheckDisabled) return;
    cancelledRef.current = false;
    checkForUpdate();
    return () => { cancelledRef.current = true; };
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

  return { status, update, error, installUpdate, retryCheck: checkForUpdate };
}
