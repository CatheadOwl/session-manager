import { useEffect } from "react";
import type { SessionMeta } from "@/types";

export type ConfirmDeleteTarget =
  | { kind: "single"; session: SessionMeta }
  | { kind: "batch"; count: number; skippedCount: number };

interface ConfirmDeleteDialogProps {
  target: ConfirmDeleteTarget;
  isDeleting: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDeleteDialog({ target, isDeleting, onConfirm, onCancel }: ConfirmDeleteDialogProps) {
  // Escape key dismisses the dialog
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !isDeleting) onCancel();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [isDeleting, onCancel]);

  const isBatch = target.kind === "batch";
  const title = isBatch
    ? `Delete ${target.count} session${target.count === 1 ? "" : "s"}?`
    : "Delete session?";
  const description = isBatch
    ? `This will move ${target.count} selected session${target.count === 1 ? "" : "s"} to your system's trash / Recycle Bin.`
    : "This will move the session to your system's trash / Recycle Bin.";
  const skippedNote =
    isBatch && target.skippedCount > 0
      ? `${target.skippedCount} read-only session(s) will be skipped.`
      : "";
  const targetLabel = isBatch
    ? `${target.count} selected session${target.count === 1 ? "" : "s"}`
    : target.session.title || target.session.sessionId;

  return (
    <div className="confirm-dialog-backdrop" role="presentation" onClick={onCancel}>
      <section
        className="confirm-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="delete-session-title"
        aria-describedby="delete-session-description"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="confirm-dialog-icon" aria-hidden="true">×</div>
        <div className="confirm-dialog-content">
          <h2 id="delete-session-title">{title}</h2>
          <p id="delete-session-description">
            {description}
            {skippedNote ? (
              <>
                <br />
                {skippedNote}
              </>
            ) : null}
          </p>
          <div className="confirm-dialog-target" title={targetLabel}>
            {targetLabel}
          </div>
        </div>
        <div className="confirm-dialog-actions">
          <button
            type="button"
            className="secondary-button"
            onClick={onCancel}
            disabled={isDeleting}
          >
            Cancel
          </button>
          <button
            type="button"
            className="danger-button"
            onClick={onConfirm}
            disabled={isDeleting}
          >
            {isDeleting ? "Deleting..." : "Delete"}
          </button>
        </div>
      </section>
    </div>
  );
}
