import { useCallback, useEffect, useRef, useState } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdateStatus = "idle" | "checking" | "available" | "downloading" | "ready" | "error";

export function useUpdater() {
  const [status, setStatus] = useState<UpdateStatus>("idle");
  const [update, setUpdate] = useState<Update | null>(null);
  const [error, setError] = useState<string | null>(null);
  const cancelledRef = useRef(false);

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
    cancelledRef.current = false;
    checkForUpdate();
    return () => { cancelledRef.current = true; };
  }, [checkForUpdate]);

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
