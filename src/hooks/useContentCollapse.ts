import { useEffect, useMemo, useState } from "react";
import type { CollapsedContent } from "@/utils/content-collapse";

export interface UseContentCollapseResult {
  expanded: boolean;
  toggle: () => void;
  displayContent: string;
  shouldCollapse: boolean;
}

/**
 * Encapsulates the common "collapse long content" pattern:
 * - Computes collapse state via a user-provided function
 * - Manages `expanded` state, resetting it when content or extra deps change
 * - Derives `displayContent` (preview vs full text)
 */
export function useContentCollapse(
  content: string,
  getCollapseState: (content: string) => CollapsedContent,
  deps?: unknown[],
): UseContentCollapseResult {
  const collapseState = useMemo(
    () => getCollapseState(content),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [content, getCollapseState],
  );
  const [expanded, setExpanded] = useState(!collapseState.shouldCollapse);

  useEffect(() => {
    setExpanded(!collapseState.shouldCollapse);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [collapseState.shouldCollapse, content, ...(deps ?? [])]);

  const toggle = () => setExpanded((prev) => !prev);

  const displayContent =
    collapseState.shouldCollapse && !expanded
      ? collapseState.previewText
      : collapseState.fullText;

  return { expanded, toggle, displayContent, shouldCollapse: collapseState.shouldCollapse };
}
