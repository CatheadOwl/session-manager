import { memo, useMemo } from "react";
import { useLatestMessage } from "@/hooks/useLatestMessage";
import type { SessionMeta } from "@/types";
import { ProviderIcon } from "@/components/ProviderIcon";
import { formatSessionTitle, formatTimestamp } from "@/utils/format";
import { getSessionKey } from "@/lib/domain";
import { useSelectionContext } from "./SessionList";
import { CopyButton } from "./CopyButton";
import { highlightText } from "./highlight";

interface SessionItemProps {
  session: SessionMeta;
  selected: boolean;
  search: string;
  onSelect: () => void;
  isSelectionSelected?: boolean;
}

export const SessionItem = memo(function SessionItem({
  session,
  selected,
  search,
  onSelect,
  isSelectionSelected = false,
}: SessionItemProps) {
  const title = useMemo(() => formatSessionTitle(session), [session]);
  const timestamp = useMemo(
    () => session.lastActiveAt ?? session.createdAt,
    [session.lastActiveAt, session.createdAt],
  );
  const { getLatestMessage } = useLatestMessage({
    providerId: session.providerId,
    sourcePath: session.sourcePath,
  });
  const selectionCtx = useSelectionContext();

  const handleClick = () => {
    if (selectionCtx?.modeRef.current) {
      selectionCtx.toggleSessionSelection(getSessionKey(session));
      onSelect(); // also show in detail panel
    } else {
      onSelect();
    }
  };

  const highlightedTitle = useMemo(
    () => highlightText(title, search),
    [title, search],
  );
  const highlightedSummary = useMemo(
    () => (session.summary ? highlightText(session.summary, search) : null),
    [session.summary, search],
  );

  return (
    <button
      type="button"
      className={`session-item ${selected ? "selected" : ""}${isSelectionSelected ? " selection-selected" : ""}`}
      onClick={handleClick}
    >
      <div className="session-item-header">
        <span className={`selection-checkbox${isSelectionSelected ? " checked" : ""}`}>{isSelectionSelected ? "✓" : ""}</span>
        <span className="session-title">{highlightedTitle}</span>
        <span className="session-provider-badge" title={session.providerId}>
          <ProviderIcon providerId={session.providerId} size={14} />
        </span>
      </div>
      {highlightedSummary ? (
        <div className="session-summary">{highlightedSummary}</div>
      ) : null}
      <div className="session-meta-row">
        <span>{formatTimestamp(timestamp)}</span>
        <span className="session-copy-latest-wrap" onClick={(e) => e.stopPropagation()}>
          <CopyButton
            getText={async () => (await getLatestMessage())?.content}
            label="Copy latest message"
            className="session-copy-latest-btn"
            disabled={!session.sourcePath}
          />
        </span>
      </div>
    </button>
  );
}, (prev, next) => {
  return (
    prev.session === next.session &&
    prev.selected === next.selected &&
    prev.search === next.search &&
    prev.isSelectionSelected === next.isSelectionSelected
  );
});

