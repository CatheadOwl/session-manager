import { memo, useMemo } from "react";
import { useContentCollapse } from "@/hooks/useContentCollapse";
import type { QaPair, SessionMessage } from "@/types";
import { formatTimestamp } from "@/utils/format";
import { getAssistantPreview } from "@/utils/content-collapse";
import { extractSystemBlocks } from "@/utils/system-blocks";
import { CopyButton } from "./CopyButton";
import { MarkdownContent } from "./MarkdownContent";
import { SystemBlockCard } from "./SessionMessageItem";
import { highlightText } from "./highlight";

interface SessionQaPairProps {
  pair: QaPair;
  messages: SessionMessage[];
  index: number;
  questionJumpIndex?: number;
  showRendered: boolean;
  tocTargetId?: string;
  tocHighlighted?: boolean;
  tocHighlightNonce?: number;
  questionSearchActive?: boolean;
  questionSearchCurrent?: boolean;
  answerSearchActive?: boolean;
  answerSearchCurrent?: boolean;
  searchQuery?: string;
}

export const SessionQaPair = memo(function SessionQaPair({
  pair,
  messages,
  index,
  questionJumpIndex,
  showRendered,
  tocTargetId,
  tocHighlighted,
  tocHighlightNonce,
  questionSearchActive,
  questionSearchCurrent,
  answerSearchActive,
  answerSearchCurrent,
  searchQuery,
}: SessionQaPairProps) {
  const question = messages[pair.questionIdx];
  const answer = messages[pair.answerIdx];

  const { text: questionText, blocks: systemBlocks } = useMemo(
    () => extractSystemBlocks(question.content),
    [question.content],
  );
  const { text: answerText, blocks: answerBlocks } = useMemo(
    () => extractSystemBlocks(answer.content),
    [answer.content],
  );
  const { expanded, toggle, displayContent, shouldCollapse } = useContentCollapse(
    answerText,
    getAssistantPreview,
    [answer.ts],
  );
  const tokenLabel = useMemo(
    () => answer.cumulativeUsage
      ? `~${Math.round(answer.cumulativeUsage.totalTokens / 1000)}k so far`
      : null,
    [answer.cumulativeUsage],
  );
  const questionSearchClass = questionSearchCurrent
    ? " search-match-current"
    : questionSearchActive
      ? " search-match"
      : "";
  const answerSearchClass = answerSearchCurrent
    ? " search-match-current"
    : answerSearchActive
      ? " search-match"
      : "";
  const trimmedSearchQuery = searchQuery?.trim() ? searchQuery : undefined;
  const answerDisplayContent = answerSearchActive ? answerText : displayContent;
  const answerCollapseClass =
    shouldCollapse && !expanded && !answerSearchActive ? " collapsed-preview" : "";

  return (
    <>
      <article
        id={tocTargetId}
        key={tocHighlighted ? tocHighlightNonce : undefined}
        className={`qa-pair${tocHighlighted ? " qa-toc-jump-highlight" : ""}`}
        data-qa-pair-index={index}
      >
      <div className="qa-pair-header">
        <span className="qa-pair-number">Pair #{index + 1}</span>
      </div>
      <div
        className={`qa-message qa-question${questionSearchClass}`}
        data-qa-question-idx={questionJumpIndex}
        data-msg-idx={pair.questionIdx}
      >
        <div className="message-header qa-message-header">
          <span className="role-badge">User</span>
          <span className="message-time">{formatTimestamp(question.ts)}</span>
          <CopyButton text={question.content} />
        </div>
        {showRendered ? (
          <div className="message-content rendered">
            <MarkdownContent
              content={questionText}
              highlightQuery={questionSearchActive ? trimmedSearchQuery : undefined}
            />
          </div>
        ) : (
          <pre className="message-content">
            {questionSearchActive && trimmedSearchQuery
              ? highlightText(questionText, trimmedSearchQuery)
              : questionText}
          </pre>
        )}
        {systemBlocks.length > 0 ? (
          <div className="system-blocks-section">
            {systemBlocks.map((block, idx) => (
              <SystemBlockCard key={idx} block={block} showRendered={showRendered} />
            ))}
          </div>
        ) : null}
      </div>
      <div
        className={`qa-message qa-answer${answerSearchClass}`}
        data-msg-idx={pair.answerIdx}
      >
        <div className="message-header qa-message-header">
          <span className="role-badge">Assistant</span>
          <span className="message-time">{formatTimestamp(answer.ts)}</span>
          <CopyButton text={answer.content} />
        </div>
        {showRendered ? (
          <div className={`message-content rendered${answerCollapseClass}`}>
            <MarkdownContent
              content={answerDisplayContent}
              highlightQuery={answerSearchActive ? trimmedSearchQuery : undefined}
            />
          </div>
        ) : (
          <pre className={`message-content${answerCollapseClass}`}>
            {answerSearchActive && trimmedSearchQuery
              ? highlightText(answerDisplayContent, trimmedSearchQuery)
              : answerDisplayContent}
          </pre>
        )}
        {shouldCollapse ? (
          <button type="button" className="link-button" onClick={toggle}>
            {expanded ? "Collapse" : "Expand"}
          </button>
        ) : null}
        {answerBlocks.length > 0 ? (
          <div className="system-blocks-section">
            {answerBlocks.map((block, idx) => (
              <SystemBlockCard key={idx} block={block} showRendered={showRendered} />
            ))}
          </div>
        ) : null}
      </div>
      </article>
      {tokenLabel ? (
        <div className="qa-token-usage-block" title="Cumulative observed input/output tokens through this pair">
          {tokenLabel}
        </div>
      ) : null}
    </>
  );
});
