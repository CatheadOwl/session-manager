import {
  type ButtonHTMLAttributes,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
  useCallback,
  useRef,
  useState,
} from "react";
import { Popover } from "./Popover";

export interface MenuProps {
  /** Accessible name (aria-label on trigger and panel). */
  label: string;
  /** Render the trigger button; Menu injects aria attrs, ref, and handlers. */
  renderTrigger: (
    triggerProps: ButtonHTMLAttributes<HTMLButtonElement> & { ref?: Ref<HTMLButtonElement> },
  ) => ReactNode;
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
 * dismissal, aria wiring, and APG-style keyboard support (arrow keys, Home/End
 * move focus between items; Escape closes and restores focus to the trigger;
 * opening with ArrowDown focuses the first item). Items should be `MenuItem`s.
 * Panel/item skins come from `.ui-panel` / `.ui-menu-item` (ui.css).
 */
export function Menu({ label, renderTrigger, align = "left", open, onOpenChange, className, children }: MenuProps) {
  const [internalOpen, setInternalOpen] = useState(false);
  const isControlled = open !== undefined;
  const isOpen = isControlled ? open : internalOpen;
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);

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

  const enabledItems = (): HTMLButtonElement[] =>
    panelRef.current
      ? Array.from(panelRef.current.querySelectorAll<HTMLButtonElement>(".ui-menu-item:not(:disabled)"))
      : [];

  const focusItem = (item: HTMLButtonElement | undefined) => item?.focus();

  const handlePanelKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const items = enabledItems();
    if (items.length === 0) return;
    const index = items.indexOf(document.activeElement as HTMLButtonElement);

    switch (event.key) {
      case "ArrowDown":
        event.preventDefault();
        focusItem(items[(index + 1 + items.length) % items.length]);
        break;
      case "ArrowUp":
        event.preventDefault();
        focusItem(items[index === -1 ? items.length - 1 : (index - 1 + items.length) % items.length]);
        break;
      case "Home":
        event.preventDefault();
        focusItem(items[0]);
        break;
      case "End":
        event.preventDefault();
        focusItem(items[items.length - 1]);
        break;
      case "Escape":
        event.preventDefault();
        setOpen(false);
        triggerRef.current?.focus();
        break;
      case "Tab":
        // Tabbing out of the menu closes it (focus moves naturally).
        setOpen(false);
        break;
    }
  };

  const handleTriggerKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (isOpen && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      event.preventDefault();
      const items = enabledItems();
      focusItem(event.key === "ArrowDown" ? items[0] : items[items.length - 1]);
    }
  };

  return (
    <div className={`ui-menu${className ? ` ${className}` : ""}`}>
      {renderTrigger({
        ref: triggerRef,
        "aria-label": label,
        "aria-haspopup": "menu",
        "aria-expanded": isOpen,
        onClick: () => setOpen(!isOpen),
        onKeyDown: handleTriggerKeyDown,
      })}
      <Popover
        open={isOpen}
        onClose={() => setOpen(false)}
        label={label}
        align={align}
        role="menu"
        panelRef={panelRef}
        onKeyDown={handlePanelKeyDown}
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
