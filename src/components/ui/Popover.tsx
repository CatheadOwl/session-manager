import { type ReactNode } from "react";
import { useClickOutside } from "@/hooks/useClickOutside";

export interface PopoverProps {
  open: boolean;
  onClose: () => void;
  /** Extra classes for the panel (consumer-specific layout/sizing). */
  className?: string;
  /** Accessible name for the panel. */
  label: string;
  align?: "left" | "right";
  role?: "dialog" | "menu";
  id?: string;
  children: ReactNode;
}

/**
 * UI primitive: an absolutely-anchored panel with outside-click dismissal.
 * The consumer wraps its trigger and this panel in one `position: relative`
 * container; the panel anchors below it (`top: calc(100% + 0.4rem)`).
 * Panel skin comes from `.ui-panel` (ui.css) — do not hand-roll panel CSS.
 */
export function Popover({
  open,
  onClose,
  className,
  label,
  align = "left",
  role = "dialog",
  id,
  children,
}: PopoverProps) {
  const ref = useClickOutside<HTMLDivElement>({ isOpen: open, onClose });

  if (!open) return null;

  return (
    <div
      ref={ref}
      id={id}
      className={`ui-panel${className ? ` ${className}` : ""}`}
      data-align={align}
      role={role}
      aria-label={label}
    >
      {children}
    </div>
  );
}
