import { memo, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { useContentCollapse } from "@/hooks/useContentCollapse";
import type { SessionMessage, ToolCallInfo, ToolResultInfo } from "@/types";
import { formatTimestamp } from "@/utils/format";
import { getAssistantPreview, getLegacyCollapsedMessage } from "@/utils/content-collapse";
import { extractSystemBlocks } from "@/utils/system-blocks";
import type { SystemBlock } from "@/utils/system-blocks";
import { getRoleDisplay } from "./role-display";
import { CopyButton } from "./CopyButton";
import { MarkdownContent } from "./MarkdownContent";
import { highlightText } from "./highlight";

// ─── JSON Syntax Highlight ───────────────────────────────────────────────

/** Try to parse a string as JSON and return highlighted React nodes. */
function tryHighlightJson(raw: string): { node: ReactNode; isJson: boolean } {
  const trimmed = raw.trim();
  if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) {
    return { node: raw, isJson: false };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(trimmed);
  } catch {
    return { node: raw, isJson: false };
  }
  const pretty = JSON.stringify(parsed, null, 2);
  return { node: tokenizeJson(pretty), isJson: true };
}

/** Tokenize pretty-printed JSON into colored spans (keys/strings/numbers/literals). */
function tokenizeJson(json: string): ReactNode[] {
  const tokens = json.match(
    /("(?:\\.|[^"\\])*"|\b(?:true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?|[{}[\]:, ]|\s+)/g,
  );
  if (!tokens) return [json];

  const parts: ReactNode[] = [];
  let idx = 0;
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (/^"/.test(t)) {
      // Key if followed by colon
      const isKey = i + 1 < tokens.length && tokens[i + 1] === ":";
      parts.push(
        <span key={idx++} className={isKey ? "json-key" : "json-string"}>
          {t}
        </span>,
      );
    } else if (/^(true|false|null)$/.test(t)) {
      parts.push(
        <span key={idx++} className="json-literal">
          {t}
        </span>,
      );
    } else if (/^-?\d/.test(t)) {
      parts.push(
        <span key={idx++} className="json-number">
          {t}
        </span>,
      );
    } else {
      // Structural chars, whitespace — no coloring
      parts.push(t);
    }
  }
  return parts;
}

// ─── Tool Result Card ────────────────────────────────────────────────────

function ToolResultCard({ toolResult }: { toolResult: ToolResultInfo }) {
  const [expanded, setExpanded] = useState(false);
  const formatted = useMemo(() => tryHighlightJson(toolResult.content), [toolResult.content]);
  return (
    <div className={`tool-result-card${expanded ? " expanded" : ""}`}>
      <button className="tool-result-toggle" type="button" onClick={() => setExpanded((v) => !v)}>
        <span className="tool-result-badge">{expanded ? "▾" : "▸"} Tool Result</span>
        {toolResult.callId ? <span className="tool-result-id">{toolResult.callId}</span> : null}
        {formatted.isJson ? <span className="json-badge">JSON</span> : null}
      </button>
      {expanded ? (
        <pre className={`tool-result-content${formatted.isJson ? " json" : ""}`}>
          {formatted.node}
        </pre>
      ) : null}
    </div>
  );
}

function ToolCallCard({ toolCall }: { toolCall: ToolCallInfo }) {
  const [expanded, setExpanded] = useState(false);
  const hasInput = toolCall.input.length > 0;
  const formatted = useMemo(
    () => (hasInput ? tryHighlightJson(toolCall.input) : { node: null, isJson: false }),
    [toolCall.input, hasInput],
  );
  return (
    <div className={`tool-call-card${expanded ? " expanded" : ""}`}>
      <button className="tool-call-toggle" type="button" onClick={() => setExpanded((v) => !v)}>
        <span className="tool-call-badge">{expanded ? "▾" : "▸"} Tool: {toolCall.name}</span>
        {toolCall.callId ? <span className="tool-call-id">{toolCall.callId}</span> : null}
        {formatted.isJson ? <span className="json-badge">JSON</span> : null}
      </button>
      {expanded && hasInput ? (
        <pre className={`tool-call-input${formatted.isJson ? " json" : ""}`}>
          {formatted.node}
        </pre>
      ) : null}
    </div>
  );
}

// ─── System Block Card ───────────────────────────────────────────────────

export function SystemBlockCard({ block, showRendered }: { block: SystemBlock; showRendered: boolean }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className={`system-block-card${expanded ? " expanded" : ""}`}>
      <button className="system-block-toggle" type="button" onClick={() => setExpanded((v) => !v)}>
        <span className="system-block-badge">{expanded ? "▾" : "▸"} System: {block.tag}</span>
      </button>
      {expanded ? (
        showRendered ? (
          <div className="system-block-content rendered">
            <MarkdownContent content={block.content} />
          </div>
        ) : (
          <pre className="system-block-content">{block.content}</pre>
        )
      ) : null}
    </div>
  );
}

interface SessionMessageItemProps {
  message: SessionMessage;
  showRendered: boolean;
  highlighted?: boolean;
  /** This message matches the active search query */
  searchActive?: boolean;
  /** This message is the current search match (scroll target) */
  searchCurrent?: boolean;
  /** Active search query for text-level highlighting (raw mode) */
  searchQuery?: string;
}

export const SessionMessageItem = memo(function SessionMessageItem({ message, showRendered, highlighted, searchActive, searchCurrent, searchQuery }: SessionMessageItemProps) {
  const isAssistant = message.role.toLowerCase() === "assistant";
  const { bodyText, systemBlocks } = useMemo(() => {
    const { text, blocks } = extractSystemBlocks(message.content);
    return { bodyText: text, systemBlocks: blocks };
  }, [message.content]);
  const collapseFn = isAssistant ? getAssistantPreview : getLegacyCollapsedMessage;
  const { expanded, toggle, displayContent, shouldCollapse } = useContentCollapse(
    bodyText,
    collapseFn,
    [message.ts],
  );

  const fullText = useMemo(() => {
    const parts: string[] = [message.content];
    if (message.toolCalls) {
      for (const tc of message.toolCalls) {
        parts.push(`\n\n--- Tool Call: ${tc.name} ---\n${tc.input}`);
      }
    }
    if (message.toolResult) {
      parts.push(`\n\n--- Tool Result ---\n${message.toolResult.content}`);
    }
    return parts.join('');
  }, [message]);

  const collapseClass = isAssistant && shouldCollapse && !expanded ? " collapsed-preview" : "";
  const searchClass = searchCurrent ? " search-match-current" : searchActive ? " search-match" : "";

  return (
    <article className={`message-item role-${getRoleDisplay(message.role).tone}${highlighted ? " fork-jump-highlight" : ""}${searchClass}`}>
      <header className="message-header">
        <span className="role-badge">{getRoleDisplay(message.role).label}</span>
        <span className="message-time">{formatTimestamp(message.ts)}</span>
        <span className="message-header-actions">
          <CopyButton text={fullText} />
        </span>
      </header>
      {showRendered ? (
        <div className={`message-content rendered${collapseClass}`}>
          <MarkdownContent content={displayContent} highlightQuery={searchQuery} />
        </div>
      ) : (
        <pre className={`message-content${collapseClass}`}>{searchQuery?.trim() ? highlightText(displayContent, searchQuery) : displayContent}</pre>
      )}
      {systemBlocks.length > 0 ? (
        <div className="system-blocks-section">
          {systemBlocks.map((block, idx) => (
            <SystemBlockCard key={idx} block={block} showRendered={showRendered} />
          ))}
        </div>
      ) : null}
      {message.toolCalls && message.toolCalls.length > 0 ? (
        <div className="tool-calls-section">
          {message.toolCalls.map((tc, idx) => (
            <ToolCallCard key={idx} toolCall={tc} />
          ))}
        </div>
      ) : null}
      {message.toolResult ? (
        <div className="tool-results-section">
          <ToolResultCard toolResult={message.toolResult} />
        </div>
      ) : null}
      {shouldCollapse ? (
        <button type="button" className="link-button" onClick={toggle}>
          {expanded ? "Collapse" : "Expand"}
        </button>
      ) : null}
    </article>
  );
});