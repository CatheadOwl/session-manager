import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { useContentCollapse } from "@/hooks/useContentCollapse";
import { useMessageSearch } from "@/hooks/useMessageSearch";
import { usePersistentState } from "@/hooks/usePersistentState";
import type { QaPair, SessionMessage, SessionMeta } from "@/types";
import { SessionMessageItem } from "./SessionMessageItem";
import { SessionQaPair } from "./SessionQaPair";
import { MessageSearchBar } from "./MessageSearchBar";
import { CopyButton } from "./CopyButton";
import { getProviderDisplay } from "./provider-display";
import { StarButton } from "./StarButton";
import { SegmentedControl } from "./SegmentedControl";
import { RawSessionContent } from "./RawSessionContent";
import { formatSessionTitle, formatTimestamp } from "@/utils/format";
import { getLegacyCollapsedMessage } from "@/utils/content-collapse";

interface SessionDetailProps {
  session: SessionMeta | null;
  messages: SessionMessage[];
  qaPairs: QaPair[];
  rawContent?: string | null;
  isLoading: boolean;
  error: Error | null;
  isDeleting: boolean;
  isArchiving: boolean;
  isRestoring: boolean;
  isStarred: boolean;
  scope: "active" | "archived";
  onToggleStar: (session: SessionMeta) => void;
  onDelete: (session: SessionMeta) => void;
  onArchive: (session: SessionMeta) => void;
  onRestore: (session: SessionMeta) => void;
  /** Jump to the Nth user event (0-indexed, text-containing only). */
  forkJumpIndex?: number;
}

export const SessionDetail = memo(function SessionDetail({
  session,
  messages,
  qaPairs,
  rawContent,
  isLoading,
  error,
  isDeleting,
  isArchiving,
  isRestoring,
  isStarred,
  scope,
  onToggleStar,
  onDelete,
  onArchive,
  onRestore,
  forkJumpIndex,
}: SessionDetailProps) {
  const [messageMode, setMessageMode] = usePersistentState<"full" | "qa">("sm:message-mode", "full");
  const [showRendered, setShowRendered] = usePersistentState("sm:show-rendered", false);
  const [metaCollapsed, setMetaCollapsed] = useState(true);
  const hasRawContent = Boolean(rawContent?.trim());
  // Fork jump: find the Nth text-containing user event in the full messages array
  const targetMsgIndex = useMemo(() => {
    if (forkJumpIndex == null || forkJumpIndex < 0) return -1;
    let userCount = 0;
    for (let i = 0; i < messages.length; i++) {
      if (messages[i].role === "user" && messages[i].content.trim()) {
        if (userCount === forkJumpIndex) return i;
        userCount++;
      }
    }
    return -1;
  }, [messages, forkJumpIndex]);

  const qaQuestionMsgIndices = useMemo(() => {
    const indices: number[] = [];
    let pendingUserIndex: number | null = null;
    let hasPendingAnswer = false;

    for (let i = 0; i < messages.length; i++) {
      switch (messages[i].role.toLowerCase()) {
        case "user":
          if (pendingUserIndex != null && hasPendingAnswer) {
            indices.push(pendingUserIndex);
          }
          pendingUserIndex = i;
          hasPendingAnswer = false;
          break;
        case "assistant":
          if (pendingUserIndex != null) {
            hasPendingAnswer = true;
          }
          break;
        default:
          break;
      }
    }

    if (pendingUserIndex != null && hasPendingAnswer) {
      indices.push(pendingUserIndex);
    }

    return indices;
  }, [messages]);

  const qaListRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLElement>(null);
  const [highlightIdx, setHighlightIdx] = useState(-1);

  // ─── Find-in-page message search ───────────────────────────────────────
  const [searchOpen, setSearchOpen] = useState(false);
  const msgSearch = useMessageSearch({ messages, enabled: searchOpen });
  const searchMatchSet = useMemo(() => new Set(msgSearch.matchIndices), [msgSearch.matchIndices]);

  // Debounce text-level highlight to avoid DOM thrashing on rapid typing
  const [debouncedQuery, setDebouncedQuery] = useState("");
  useEffect(() => {
    const q = searchOpen ? msgSearch.query : "";
    if (!q.trim()) { setDebouncedQuery(""); return; }
    const timer = setTimeout(() => setDebouncedQuery(q), 150);
    return () => clearTimeout(timer);
  }, [msgSearch.query, searchOpen]);

  const closeSearch = useCallback(() => {
    setSearchOpen(false);
    msgSearch.clear();
    setDebouncedQuery("");
  }, [msgSearch]);

  const rowVirtualizer = useVirtualizer({
    count: messageMode === "full" ? messages.length : 0,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 200,
    overscan: 5,
  });

  // Scroll to current match whenever it changes
  useEffect(() => {
    const idx = msgSearch.currentMsgIndex;
    if (idx < 0) return;
    if (messageMode === "full") {
      rowVirtualizer.scrollToIndex(idx, { align: "center" });
    } else if (qaListRef.current) {
      // QA mode: find the QA pair whose question is at or before this message
      const el = qaListRef.current.querySelector(`[data-qa-question-idx="${idx}"]`)
        ?? qaListRef.current.querySelector(`[data-msg-idx="${idx}"]`);
      el?.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, [msgSearch.currentMsgIndex, msgSearch.currentMatch, messageMode, rowVirtualizer]);

  const scrollToTop = useCallback(() => {
    if (messageMode === "full") {
      rowVirtualizer.scrollToIndex(0, { align: "start", behavior: "instant" });
    } else {
      scrollRef.current?.scrollTo({ top: 0, behavior: "smooth" });
    }
  }, [messageMode, rowVirtualizer]);

  const scrollToBottom = useCallback(() => {
    if (messageMode === "full") {
      rowVirtualizer.scrollToIndex(messages.length - 1, { align: "end", behavior: "instant" });
    } else {
      const el = scrollRef.current;
      if (el) el.scrollTo({ top: el.scrollHeight, behavior: "smooth" });
    }
  }, [messageMode, rowVirtualizer, messages.length]);

  useEffect(() => {
    if (targetMsgIndex < 0) return;

    if (messageMode === "full") {
      // Virtualized full mode: scrollToIndex + state-based highlight
      rowVirtualizer.scrollToIndex(targetMsgIndex, { align: "center" });
      setHighlightIdx(targetMsgIndex);
      const timer = setTimeout(() => setHighlightIdx(-1), 2500);
      return () => clearTimeout(timer);
    }

    // QA mode: DOM-based smooth scroll (fewer elements, stable layout)
    if (!qaListRef.current) return;
    const el = qaListRef.current.querySelector(`[data-qa-question-idx="${targetMsgIndex}"]`);
    if (!el) return;
    let raf2 = 0;
    const raf1 = requestAnimationFrame(() => {
      raf2 = requestAnimationFrame(() => {
        el.scrollIntoView({ behavior: "smooth", block: "center" });
        el.classList.add("fork-jump-highlight");
      });
    });
    const timer = setTimeout(() => el.classList.remove("fork-jump-highlight"), 2500);
    return () => {
      cancelAnimationFrame(raf1);
      cancelAnimationFrame(raf2);
      clearTimeout(timer);
    };
  }, [targetMsgIndex, messageMode, messages, qaPairs]);

  if (!session) {
    return (
      <main className="session-detail empty-detail">
        <div className="empty-state large">
          <strong>Select a session</strong>
          <span>Choose a session from the list to inspect messages.</span>
        </div>
      </main>
    );
  }

  const sessionTitle = formatSessionTitle(session);

  return (
    <main className="session-detail">
      <div className="detail-fixed">
        <header className="detail-header">
          <div className="detail-title-block">
            <div className="detail-provider-row">
              <span className="provider-pill">{getProviderDisplay(session.providerId).label}</span>
              <div className="detail-actions">
                <StarButton starred={isStarred} onToggle={() => onToggleStar(session)} />
                {scope === "active" && (
                  <button
                    type="button"
                    className="secondary-button"
                    disabled={isArchiving}
                    onClick={() => onArchive(session)}
                  >
                    {isArchiving ? "Archiving..." : "Archive"}
                  </button>
                )}
                {scope === "archived" && (
                  <button
                    type="button"
                    className="secondary-button"
                    disabled={isRestoring}
                    onClick={() => onRestore(session)}
                  >
                    {isRestoring ? "Restoring..." : "Restore"}
                  </button>
                )}
                <button
                  type="button"
                  className="danger-button"
                  disabled={isDeleting}
                  onClick={() => onDelete(session)}
                >
                  {isDeleting ? "Deleting..." : "Delete"}
                </button>
              </div>
            </div>
            <h2 title={sessionTitle}>{sessionTitle}</h2>
          </div>
        </header>

        <section className={`metadata-card${metaCollapsed ? " collapsed" : ""}`}>
          <button
            type="button"
            className="metadata-toggle"
            onClick={() => setMetaCollapsed((c) => !c)}
            aria-expanded={!metaCollapsed}
          >
            Metadata
          </button>
          <div className="metadata-body">
            <dl>
              <div>
                <dt>Created</dt>
                <dd>{formatTimestamp(session.createdAt)}</dd>
              </div>
              <div>
                <dt>Last active</dt>
                <dd>{formatTimestamp(session.lastActiveAt ?? session.createdAt)}</dd>
              </div>
              <div>
                <dt>Project path</dt>
                <dd>{session.projectDir || "—"}</dd>
              </div>
              <div>
                <dt>Source path</dt>
                <dd>{session.sourcePath || "—"}</dd>
              </div>
              <div>
                <dt>Session ID</dt>
                <dd>{session.sessionId}</dd>
              </div>
              <div>
                <dt>Resume</dt>
                <dd>{session.resumeCommand || "—"}</dd>
              </div>
            </dl>
          </div>
        </section>
      </div>

      <section className="messages-section" ref={scrollRef}>
        <div className="messages-sticky-group">
        <div className="messages-header">
          <h3>Messages</h3>
          <span>{isLoading ? "Loading..." : `${messages.length} messages`}</span>
          <div className="messages-header-actions">
            <button
              type="button"
              className={`ghost-button search-toggle-btn${searchOpen ? " active" : ""}`}
              onClick={() => (searchOpen ? closeSearch() : setSearchOpen(true))}
              title="Search in messages"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/></svg>
            </button>
            <button
              type="button"
              className="ghost-button scroll-nav-btn"
              onClick={scrollToTop}
              title="Jump to top"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><path d="m17 11-5-5-5 5"/><path d="m17 18-5-5-5 5"/></svg>
            </button>
            <button
              type="button"
              className="ghost-button scroll-nav-btn"
              onClick={scrollToBottom}
              title="Jump to bottom"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><path d="m7 13 5 5 5-5"/><path d="m7 6 5 5 5-5"/></svg>
            </button>
            <button
              type="button"
              className="ghost-button md-toggle"
              onClick={() => setShowRendered((v) => !v)}
              title={showRendered ? "Show raw text" : "Render markdown"}
            >
              {showRendered ? "Raw" : "MD"}
            </button>
            <SegmentedControl
              options={[{ value: "full" as const, label: "Full" }, { value: "qa" as const, label: "Q&A" }]}
              value={messageMode}
              onChange={(v) => setMessageMode(v)}
            />
          </div>
        </div>
        {searchOpen && (
          <MessageSearchBar
            query={msgSearch.query}
            onQueryChange={msgSearch.setQuery}
            matchCount={msgSearch.matchIndices.length}
            currentMatch={msgSearch.currentMatch}
            onNext={msgSearch.goNext}
            onPrev={msgSearch.goPrev}
            onClose={closeSearch}
          />
        )}
        </div>
        {error ? <div className="error-box">{error.message}</div> : null}
        {!isLoading && messages.length === 0 ? (
          hasRawContent ? (
            <RawSessionContent content={rawContent ?? ""} />
          ) : (
            <div className="empty-state">No messages found for this session.</div>
          )
        ) : null}
        {messages.length > 0 ? (
          messageMode === "qa" ? (
            <div className="message-list qa-list" ref={qaListRef}>
              {qaPairs.length === 0 ? (
                <div className="empty-state">
                  No complete Q&A pairs found. Single-turn sessions have only a user message.
                </div>
              ) : (
                qaPairs.map((pair, index) => (
                  <SessionQaPair
                    key={index}
                    pair={pair}
                    messages={messages}
                    index={index}
                    questionJumpIndex={qaQuestionMsgIndices[index]}
                    showRendered={showRendered}
                  />
                ))
              )}
            </div>
          ) : (
            <div
              className="message-list message-list--virtual"
              style={{ height: `${rowVirtualizer.getTotalSize()}px`, position: "relative" }}
            >
              {rowVirtualizer.getVirtualItems().map((virtualRow) => {
                const message = messages[virtualRow.index];
                return (
                  <div
                    key={`${message.ts ?? "no-ts"}-${virtualRow.index}`}
                    data-index={virtualRow.index}
                    data-msg-idx={virtualRow.index}
                    ref={rowVirtualizer.measureElement}
                    className="message-wrapper"
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      width: "100%",
                      transform: `translateY(${virtualRow.start}px)`,
                    }}
                  >
                    <SessionMessageItem
                      message={message}
                      showRendered={showRendered}
                      highlighted={virtualRow.index === highlightIdx}
                      searchActive={searchOpen && searchMatchSet.has(virtualRow.index)}
                      searchCurrent={searchOpen && virtualRow.index === msgSearch.currentMsgIndex}
                      searchQuery={debouncedQuery}
                    />
                  </div>
                );
              })}
            </div>
          )
        ) : null}
      </section>
    </main>
  );
});
