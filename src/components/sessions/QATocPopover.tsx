import { useCallback, useEffect, useRef, useState } from "react";
import { List } from "lucide-react";
import { useClickOutside } from "@/hooks/useClickOutside";

export interface QATocItem {
  id: string;
  label: string;
}

interface QATocPopoverProps {
  items: QATocItem[];
  initialIndex: number;
  selectedIndex: number;
  onOpen: () => void;
  onJump: (index: number) => void;
}

export function QATocPopover({ items, initialIndex, selectedIndex, onOpen, onJump }: QATocPopoverProps) {
  const [open, setOpen] = useState(false);
  const popoverRef = useClickOutside<HTMLDivElement>({
    isOpen: open,
    onClose: () => setOpen(false),
  });
  const listRef = useRef<HTMLDivElement | null>(null);
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([]);

  const handleToggle = useCallback(() => {
    if (open) {
      setOpen(false);
      return;
    }
    onOpen();
    setOpen(true);
  }, [onOpen, open]);

  const handleJump = useCallback((index: number) => {
    onJump(index);
    setOpen(false);
  }, [onJump]);

  useEffect(() => {
    if (!open) return;
    const targetIndex = initialIndex >= 0 ? initialIndex : 0;
    requestAnimationFrame(() => {
      const listEl = listRef.current;
      const itemEl = itemRefs.current[targetIndex];
      if (!listEl || !itemEl) return;
      const itemTop = itemEl.offsetTop - listEl.offsetTop;
      const nextScrollTop = itemTop - (listEl.clientHeight - itemEl.offsetHeight) / 2;
      listEl.scrollTop = Math.max(0, nextScrollTop);
    });
  }, [initialIndex, open]);

  if (items.length <= 1) return null;

  return (
    <div className="qa-toc-popover" ref={popoverRef}>
      {open ? (
        <div className="qa-toc-panel" role="dialog" aria-label="Q&A table of contents">
          <div className="qa-toc-list" ref={listRef} aria-label="Q&A questions">
            {items.map((item, index) => (
              <button
                key={item.id}
                ref={(node) => { itemRefs.current[index] = node; }}
                type="button"
                className={`qa-toc-item${index === selectedIndex ? " active" : ""}`}
                onClick={() => handleJump(index)}
                title={item.label}
                aria-current={index === selectedIndex ? "true" : undefined}
              >
                <span className="qa-toc-item-text">{item.label}</span>
              </button>
            ))}
          </div>
        </div>
      ) : null}
      <button
        type="button"
        className={`qa-toc-trigger${open ? " active" : ""}`}
        onClick={handleToggle}
        aria-label={open ? "Close Q&A table of contents" : "Open Q&A table of contents"}
        title={open ? "Close Q&A table of contents" : "Open Q&A table of contents"}
        aria-expanded={open}
      >
        <List aria-hidden="true" size={16} strokeWidth={2.2} />
      </button>
    </div>
  );
}
