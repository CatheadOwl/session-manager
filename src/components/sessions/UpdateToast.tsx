import { memo } from "react";
import { Download, RefreshCw } from "lucide-react";
import type { UpdateStatus } from "@/hooks/useUpdater";

interface UpdateToastProps {
  status: UpdateStatus;
  version?: string;
  onInstall: () => void;
  onRetry: () => void;
}

/** Compact update button shown on the collapsed folder toolbar. */
export const UpdateToast = memo(function UpdateToast({
  status,
  version,
  onInstall,
  onRetry,
}: UpdateToastProps) {
  // Only show when there's something actionable
  if (status === "idle") return null;

  const busy = status === "checking" || status === "downloading";
  const handleClick = status === "error" ? onRetry : onInstall;

  return (
    <button
      type="button"
      className={`update-collapsed-btn${busy ? " spinning" : ""}`}
      onClick={handleClick}
      disabled={busy}
      title={
        status === "checking"
          ? "Checking for updates…"
          : status === "downloading"
            ? "Downloading update…"
            : status === "error"
              ? "Update failed – click to retry"
              : `Update to v${version ?? "?"}`
      }
    >
      {busy ? <RefreshCw size={14} /> : <Download size={14} />}
      {!busy && <span className="update-dot" />}
    </button>
  );
});
