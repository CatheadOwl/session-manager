import { memo, useEffect } from "react";
import { CheckCircle2, X, XCircle } from "lucide-react";
import type { QaExportStatus } from "@/hooks/useQaExport";

interface ExportToastProps {
  status: QaExportStatus;
  onDismiss: () => void;
}

/** Auto-dismiss delay for success results (ms). Errors stay until dismissed. */
const DONE_AUTO_HIDE_MS = 6000;

/**
 * Floating bottom-right notification for Q&A export results, following the
 * transient-notification pattern of UpdateToast: show only when there is
 * something to say, never nag. Success auto-hides; errors persist until the
 * user dismisses them. The "exporting" state is conveyed by the toolbar
 * button, not by this toast.
 */
export const ExportToast = memo(function ExportToast({ status, onDismiss }: ExportToastProps) {
  const visible = status.state === "done" || status.state === "error";

  useEffect(() => {
    if (status.state !== "done") return;
    const timer = window.setTimeout(onDismiss, DONE_AUTO_HIDE_MS);
    return () => window.clearTimeout(timer);
  }, [status.state, status.message, onDismiss]);

  if (!visible) return null;

  const isError = status.state === "error";

  return (
    <div
      className={`qa-export-toast${isError ? " qa-export-toast-error" : ""}`}
      role={isError ? "alert" : "status"}
    >
      {isError ? (
        <XCircle size={16} aria-hidden="true" />
      ) : (
        <CheckCircle2 size={16} aria-hidden="true" />
      )}
      <span className="qa-export-toast-message" title={status.message}>
        {status.message}
      </span>
      <button
        type="button"
        className="qa-export-toast-close"
        onClick={onDismiss}
        aria-label="Dismiss notification"
      >
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  );
});
