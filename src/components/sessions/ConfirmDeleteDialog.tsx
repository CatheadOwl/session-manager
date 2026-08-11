import { useEffect } from "react";
import type { SessionMeta } from "@/types";

export type ConfirmActionTarget =
  | { kind: "delete-single"; session: SessionMeta }
  | { kind: "delete-batch"; count: number; skippedCount: number }
  | {
      kind: "folder-lifecycle";
      action: "archive" | "restore";
      folder: string;
      count: number;
      skippedCount: number;
    };

interface ConfirmActionDialogProps {
  target: ConfirmActionTarget;
  isWorking: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmActionDialog({ target, isWorking, onConfirm, onCancel }: ConfirmActionDialogProps) {
  // Escape key dismisses the dialog
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !isWorking) onCancel();
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [isWorking, onCancel]);

  const copy = getConfirmActionCopy(target, isWorking);
  const skippedNote =
    "skippedCount" in target && target.skippedCount > 0
      ? `${target.skippedCount} read-only session(s) will be skipped.`
      : "";

  return (
    <div
      className="confirm-dialog-backdrop"
      role="presentation"
      onClick={() => {
        if (!isWorking) onCancel();
      }}
    >
      <section
        className="confirm-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="confirm-action-title"
        aria-describedby="confirm-action-description"
        onClick={(event) => event.stopPropagation()}
      >
        <div className={`confirm-dialog-icon ${copy.tone}`} aria-hidden="true">{copy.icon}</div>
        <div className="confirm-dialog-content">
          <h2 id="confirm-action-title">{copy.title}</h2>
          <p id="confirm-action-description">
            {copy.description}
            {skippedNote ? (
              <>
                <br />
                {skippedNote}
              </>
            ) : null}
          </p>
          <div className="confirm-dialog-target" title={copy.targetLabel}>
            {copy.targetLabel}
          </div>
        </div>
        <div className="confirm-dialog-actions">
          <button
            type="button"
            className="secondary-button"
            onClick={onCancel}
            disabled={isWorking}
          >
            Cancel
          </button>
          <button
            type="button"
            className={copy.confirmClassName}
            onClick={onConfirm}
            disabled={isWorking}
          >
            {isWorking ? copy.workingLabel : copy.confirmLabel}
          </button>
        </div>
      </section>
    </div>
  );
}

function getConfirmActionCopy(target: ConfirmActionTarget, isWorking: boolean) {
  if (target.kind === "delete-single") {
    return {
      title: "Delete session?",
      description: "This will move the session to your system's trash / Recycle Bin.",
      targetLabel: target.session.title || target.session.sessionId,
      confirmLabel: "Delete",
      workingLabel: isWorking ? "Deleting..." : "Delete",
      confirmClassName: "danger-button",
      icon: "×",
      tone: "danger",
    };
  }

  if (target.kind === "delete-batch") {
    return {
      title: `Delete ${target.count} session${target.count === 1 ? "" : "s"}?`,
      description: `This will move ${target.count} selected session${target.count === 1 ? "" : "s"} to your system's trash / Recycle Bin.`,
      targetLabel: `${target.count} selected session${target.count === 1 ? "" : "s"}`,
      confirmLabel: "Delete",
      workingLabel: isWorking ? "Deleting..." : "Delete",
      confirmClassName: "danger-button",
      icon: "×",
      tone: "danger",
    };
  }

  const isArchive = target.action === "archive";
  const verb = isArchive ? "Archive" : "Restore";
  return {
    title: `${verb} ${target.count} session${target.count === 1 ? "" : "s"}?`,
    description: `${verb} sessions from this folder. Pinned folders are not changed.`,
    targetLabel: target.folder,
    confirmLabel: verb,
    workingLabel: isArchive ? "Archiving..." : "Restoring...",
    confirmClassName: isArchive ? "danger-button" : "secondary-button",
    icon: isArchive ? "↓" : "↑",
    tone: isArchive ? "danger" : "neutral",
  };
}

export { ConfirmActionDialog as ConfirmDeleteDialog };
export type { ConfirmActionTarget as ConfirmDeleteTarget };
