import { useEffect, useRef } from "react";
import type { KeyboardEvent } from "react";

interface MessageSearchBarProps {
  query: string;
  onQueryChange: (q: string) => void;
  matchCount: number;
  currentMatch: number;
  onNext: () => void;
  onPrev: () => void;
  onClose: () => void;
}

/**
 * Find-in-page style search bar for the messages area.
 * Enter → next match, Shift+Enter → previous, Escape → close.
 */
export function MessageSearchBar({
  query,
  onQueryChange,
  matchCount,
  currentMatch,
  onNext,
  onPrev,
  onClose,
}: MessageSearchBarProps) {
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  const handleKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") {
      e.preventDefault();
      if (e.shiftKey) onPrev();
      else onNext();
    } else if (e.key === "Escape") {
      e.preventDefault();
      onClose();
    }
  };

  const hasQuery = query.trim().length > 0;

  return (
    <div className="message-search-bar">
      <div className="message-search-input-wrap">
        <svg className="message-search-icon" width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="11" cy="11" r="8" />
          <path d="m21 21-4.3-4.3" />
        </svg>
        <input
          ref={inputRef}
          type="text"
          className="message-search-input"
          placeholder="Search in messages..."
          value={query}
          onChange={(e) => onQueryChange(e.target.value)}
          onKeyDown={handleKeyDown}
          spellCheck={false}
        />
        {hasQuery && (
          <span className={`message-search-count${matchCount === 0 ? " no-results" : ""}`}>
            {matchCount === 0 ? "0 results" : `${currentMatch + 1}/${matchCount}`}
          </span>
        )}
      </div>
      <div className="message-search-nav">
        <button
          type="button"
          className="ghost-button search-nav-btn"
          onClick={onPrev}
          disabled={matchCount === 0}
          title="Previous match (Shift+Enter)"
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round"><path d="m18 15-6-6-6 6"/></svg>
        </button>
        <button
          type="button"
          className="ghost-button search-nav-btn"
          onClick={onNext}
          disabled={matchCount === 0}
          title="Next match (Enter)"
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round"><path d="m6 9 6 6 6-6"/></svg>
        </button>
        <button
          type="button"
          className="ghost-button search-nav-btn"
          onClick={onClose}
          title="Close search (Escape)"
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>
        </button>
      </div>
    </div>
  );
}
