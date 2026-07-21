import { useEffect, useRef, useState } from "react";
import { copyText } from "@/utils/clipboard";

type CopyStatus = "idle" | "copied" | "failed";

interface UseCopyFeedbackOptions {
  resetAfterMs?: number;
}

export function useCopyFeedback(options?: UseCopyFeedbackOptions) {
  const resetAfterMs = options?.resetAfterMs ?? 1400;
  const [status, setStatus] = useState<CopyStatus>("idle");
  const resetTimerRef = useRef<number | null>(null);

  useEffect(() => {
    return () => {
      if (resetTimerRef.current != null) {
        window.clearTimeout(resetTimerRef.current);
      }
    };
  }, []);

  const scheduleReset = () => {
    if (resetTimerRef.current != null) {
      window.clearTimeout(resetTimerRef.current);
    }
    resetTimerRef.current = window.setTimeout(() => {
      setStatus("idle");
      resetTimerRef.current = null;
    }, resetAfterMs);
  };

  const copyWithFeedback = async (text?: string | null) => {
    try {
      await copyText(text);
      setStatus("copied");
    } catch {
      setStatus("failed");
    } finally {
      scheduleReset();
    }
  };

  return {
    status,
    copyWithFeedback,
    labelFor: (idleLabel: string) => {
      if (status === "copied") return "✓ Copied";
      if (status === "failed") return "Copy failed";
      return idleLabel;
    },
  };
}
