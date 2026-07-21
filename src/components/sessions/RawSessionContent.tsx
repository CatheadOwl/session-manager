import { useContentCollapse } from "@/hooks/useContentCollapse";
import { getLegacyCollapsedMessage } from "@/utils/content-collapse";
import { CopyButton } from "./CopyButton";

interface RawSessionContentProps {
  content: string;
}

export function RawSessionContent({ content }: RawSessionContentProps) {
  const { expanded, toggle, displayContent, shouldCollapse } = useContentCollapse(
    content,
    getLegacyCollapsedMessage,
  );

  return (
    <article className="message-item role-unknown raw-session-content">
      <header className="message-header">
        <div>
          <span className="role-badge">Raw session content</span>
          <p className="raw-session-note">
            No standard parsed messages were found for this session.
          </p>
        </div>
        <CopyButton text={content} />
      </header>
      <pre className="message-content">{displayContent}</pre>
      {shouldCollapse ? (
        <button type="button" className="link-button" onClick={toggle}>
          {expanded ? "Collapse" : "Expand"}
        </button>
      ) : null}
    </article>
  );
}
