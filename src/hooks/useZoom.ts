import { useEffect } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { usePersistentState } from "./usePersistentState";

const MIN_ZOOM = 0.5;
const MAX_ZOOM = 2.0;
const STEP = 0.1;

function clamp(value: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round(value * 10) / 10));
}

/**
 * Enables Ctrl+Wheel zoom (and Ctrl+=/Ctrl+-/Ctrl+0 keyboard shortcuts)
 * using Tauri's native WebView zoom API. Persists the zoom level.
 */
export function useZoom() {
  const [zoom, setZoom] = usePersistentState<number>("app-zoom-level", 1);

  // Apply zoom to webview whenever it changes
  useEffect(() => {
    void getCurrentWebview().setZoom(zoom);
  }, [zoom]);

  // Ctrl+Wheel handler
  useEffect(() => {
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey) return;
      e.preventDefault();
      const delta = e.deltaY > 0 ? -STEP : STEP;
      setZoom((prev) => clamp(prev + delta));
    };

    // Must use { passive: false } to allow preventDefault
    document.addEventListener("wheel", onWheel, { passive: false });
    return () => document.removeEventListener("wheel", onWheel);
  }, [setZoom]);

  // Ctrl+= / Ctrl+- / Ctrl+0 keyboard shortcuts
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (!e.ctrlKey) return;

      if (e.key === "=" || e.key === "+") {
        e.preventDefault();
        setZoom((prev) => clamp(prev + STEP));
      } else if (e.key === "-") {
        e.preventDefault();
        setZoom((prev) => clamp(prev - STEP));
      } else if (e.key === "0") {
        e.preventDefault();
        setZoom(1);
      }
    };

    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [setZoom]);
}
