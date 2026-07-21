import { useEffect, useRef } from "react";

interface UseClickOutsideOptions {
  isOpen: boolean;
  onClose: () => void;
}

/**
 * Hook that listens for clicks outside the ref element and Escape key.
 * Calls onClose when either is detected.
 * Returns a ref to attach to the element to watch.
 */
export function useClickOutside<T extends HTMLElement>({ isOpen, onClose }: UseClickOutsideOptions) {
  const ref = useRef<T | null>(null);

  useEffect(() => {
    if (!isOpen) return;

    const handlePointerDown = (event: PointerEvent) => {
      if (!ref.current?.contains(event.target as Node)) {
        onClose();
      }
    };

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        onClose();
      }
    };

    document.addEventListener("pointerdown", handlePointerDown);
    document.addEventListener("keydown", handleKeyDown);

    return () => {
      document.removeEventListener("pointerdown", handlePointerDown);
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [isOpen, onClose]);

  return ref;
}
