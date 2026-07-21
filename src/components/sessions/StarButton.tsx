import { Star } from "lucide-react";

interface StarButtonProps {
  starred: boolean;
  onToggle: () => void;
}

export function StarButton({ starred, onToggle }: StarButtonProps) {
  return (
    <button
      type="button"
      className={`ghost-button star-btn${starred ? " active" : ""}`}
      onClick={(e) => { e.stopPropagation(); onToggle(); }}
      aria-pressed={starred}
      aria-label={starred ? "Unstar session" : "Star session"}
      title={starred ? "Unstar session" : "Star session"}
    >
      <Star aria-hidden="true" size={20} strokeWidth={2.4} />
    </button>
  );
}
