import { type ButtonHTMLAttributes, type ReactNode, useCallback, useState } from "react";
import { Popover } from "./Popover";

export interface MenuProps {
  /** Accessible name (aria-label on trigger and panel). */
  label: string;
  /** Render the trigger button; Menu injects aria attrs and click handling. */
  renderTrigger: (triggerProps: ButtonHTMLAttributes<HTMLButtonElement>) => ReactNode;
  align?: "left" | "right";
  /** Controlled open state; omit for uncontrolled. */
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
  /** Extra classes for the wrapper (e.g. layout sizing like flex:1). */
  className?: string;
  children: ReactNode;
}

/**
 * UI primitive: trigger + anchored menu panel (role="menu") with outside-click
 * dismissal and aria wiring. Items should be `MenuItem`s. Panel/item skins
 * come from `.ui-panel` / `.ui-menu-item` (ui.css).
 */
export function Menu({ label, renderTrigger, align = "left", open, onOpenChange, className, children }: MenuProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const isControlled = open !== undefined;
  const isOpen = isControlled ? open : internalOpen;

  const setOpen = useCallback(
    (next: boolean) => {
      if (isControlled) {
        onOpenChange?.(next);
        return;
      }
      setInternalOpen(next);
    },
    [isControlled, onOpenChange],
  );

  return (
    <div className={`ui-menu${className ? ` ${className}` : ""}`}>
      {renderTrigger({
        "aria-label": label,
        "aria-haspopup": "menu",
        "aria-expanded": isOpen,
        onClick: () => setOpen(!isOpen),
      })}
      <Popover
        open={isOpen}
        onClose={() => setOpen(false)}
        label={label}
        align={align}
        role="menu"
      >
        {children}
      </Popover>
    </div>
  );
}

export interface MenuItemProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** Radio semantics for single-choice menus. */
  checked?: boolean;
  active?: boolean;
}

/** UI primitive: one menu row. `checked` renders menuitemradio semantics. */
export function MenuItem({ checked, active, className, children, ...rest }: MenuItemProps) {
  return (
    <button
      type="button"
      role={checked === undefined ? "menuitem" : "menuitemradio"}
      aria-checked={checked}
      className={`ui-menu-item${active ? " active" : ""}${className ? ` ${className}` : ""}`}
      {...rest}
    >
      {children}
    </button>
  );
}
