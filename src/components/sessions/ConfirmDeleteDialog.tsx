import { useEffect } from "react";
import type { SessionMeta } from "@/types";

interface ConfirmDeleteDialogProps {
  session: SessionMeta;
  isDeleting: boolean;
  onConfirm: (session: SessionMeta) => void;
  onCancel: () => void;
}

export function ConfirmDeleteDialog({ session, isDeleting, onConfirm, onCancel }: ConfirmDeleteDialogProps) {
  // Escape key dismisses the dialog
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !isDeleting) onCancel();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [isDeleting, onCancel]);

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
          <h2 id="delete-session-title">Delete session?</h2>
          <p id="delete-session-description">
            This will move the session to your system's trash / Recycle Bin.
          </p>
          <div className="confirm-dialog-target" title={session.title || session.sessionId}>
            {session.title || session.sessionId}
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
            onClick={() => onConfirm(session)}
            disabled={isDeleting}
          >
            {isDeleting ? "Deleting..." : "Delete"}
          </button>
        </div>
      </section>
    </div>
  );
}
