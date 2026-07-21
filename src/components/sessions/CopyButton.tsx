import { Check, Copy, X } from "lucide-react";
import { useCopyFeedback } from "@/hooks/useCopyFeedback";

interface CopyButtonProps {
  /** Static text to copy, or an async getter that resolves the text on click. */
  text?: string;
  getText?: () => Promise<string | undefined | null>;
  label?: string;
  className?: string;
  disabled?: boolean;
}

export function CopyButton({ text, getText, label = "Copy", className, disabled }: CopyButtonProps) {
  const copyFeedback = useCopyFeedback();
  const Icon = copyFeedback.status === "copied" ? Check : copyFeedback.status === "failed" ? X : Copy;
  const statusLabel = copyFeedback.status === "copied"
    ? "Copied"
    : copyFeedback.status === "failed"
      ? "Copy failed"
      : label;

  const handleClick = async () => {
    if (getText) {
      const resolved = await getText();
      await copyFeedback.copyWithFeedback(resolved);
    } else {
      await copyFeedback.copyWithFeedback(text);
    }
  };

  return (
    <button
      type="button"
      className={`ghost-button copy-feedback-button copy-feedback-${copyFeedback.status}${className ? ` ${className}` : ""}`}
      onClick={() => void handleClick()}
      disabled={disabled}
      aria-label={statusLabel}
      title={statusLabel}
    >
      <Icon aria-hidden="true" size={14} strokeWidth={2} />
    </button>
  );
}
