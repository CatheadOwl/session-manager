import { memo } from "react";
import { Download, RefreshCw } from "lucide-react";
import type { UpdateStatus } from "@/hooks/useUpdater";

interface UpdateToastProps {
  status: UpdateStatus;
  version?: string;
  onInstall: () => void;
}

/** Compact update button shown on the collapsed folder toolbar. */
export const UpdateToast = memo(function UpdateToast({
  status,
  version,
  onInstall,
}: UpdateToastProps) {
  // Only show when there's something actionable.
  // `error` is intentionally silent: a failed check (network hiccup, release
  // not yet published) should not nag the user — it auto-retries on next launch.
  if (status === "idle" || status === "error") return null;

  const busy = status === "checking" || status === "downloading";

  return (
    <button
      type="button"
      className={`update-collapsed-btn${busy ? " spinning" : ""}`}
      onClick={onInstall}
      disabled={busy}
      title={
        status === "checking"
          ? "Checking for updates…"
          : status === "downloading"
            ? "Downloading update…"
            : `Update to v${version ?? "?"}`
      }
    >
      {busy ? <RefreshCw size={14} /> : <Download size={14} />}
      {!busy && <span className="update-dot" />}
    </button>
  );
});
