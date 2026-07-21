import { memo, useMemo } from "react";
import { useContentCollapse } from "@/hooks/useContentCollapse";
import type { QaPair, SessionMessage } from "@/types";
import { formatTimestamp } from "@/utils/format";
import { getAssistantPreview } from "@/utils/content-collapse";
import { extractSystemBlocks } from "@/utils/system-blocks";
import { CopyButton } from "./CopyButton";
import { MarkdownContent } from "./MarkdownContent";
import { SystemBlockCard } from "./SessionMessageItem";

interface SessionQaPairProps {
  pair: QaPair;
  messages: SessionMessage[];
  index: number;
  questionJumpIndex?: number;
  showRendered: boolean;
}

export const SessionQaPair = memo(function SessionQaPair({ pair, messages, index, questionJumpIndex, showRendered }: SessionQaPairProps) {
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

  return (
    <>
      <article className="qa-pair">
      <div className="qa-pair-header">
        <span className="qa-pair-number">Pair #{index + 1}</span>
      </div>
      <div className="qa-message qa-question" data-qa-question-idx={questionJumpIndex}>
        <div className="message-header qa-message-header">
          <span className="role-badge">User</span>
          <span className="message-time">{formatTimestamp(question.ts)}</span>
          <CopyButton text={question.content} />
        </div>
        {showRendered ? (
          <div className="message-content rendered">
            <MarkdownContent content={questionText} />
          </div>
        ) : (
          <pre className="message-content">{questionText}</pre>
        )}
        {systemBlocks.length > 0 ? (
          <div className="system-blocks-section">
            {systemBlocks.map((block, idx) => (
              <SystemBlockCard key={idx} block={block} showRendered={showRendered} />
            ))}
          </div>
        ) : null}
      </div>
      <div className="qa-message qa-answer">
        <div className="message-header qa-message-header">
          <span className="role-badge">Assistant</span>
          <span className="message-time">{formatTimestamp(answer.ts)}</span>
          <CopyButton text={answer.content} />
        </div>
        {showRendered ? (
          <div className={`message-content rendered${shouldCollapse && !expanded ? " collapsed-preview" : ""}`}>
            <MarkdownContent content={displayContent} />
          </div>
        ) : (
          <pre className={`message-content${shouldCollapse && !expanded ? " collapsed-preview" : ""}`}>
            {displayContent}
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