import { memo, useEffect, useRef } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import remarkBreaks from "remark-breaks";
import type { Components } from "react-markdown";

interface MarkdownContentProps {
  content: string;
  /** Optional search query for text-level highlighting in rendered output */
  highlightQuery?: string;
}

const components: Components = {
  a: ({ href, children }) => (
    <a href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ),
};

/** Walk text nodes under root and wrap case-insensitive matches of needle in <mark>. */
function highlightDom(root: HTMLElement, needle: string) {
  const lowerNeedle = needle.toLowerCase();
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const textNodes: Text[] = [];
  while (walker.nextNode()) {
    textNodes.push(walker.currentNode as Text);
  }
  for (const node of textNodes) {
    const text = node.nodeValue ?? "";
    const lower = text.toLowerCase();
    if (!lower.includes(lowerNeedle)) continue;

    const frag = document.createDocumentFragment();
    let cursor = 0;
    let idx = lower.indexOf(lowerNeedle, cursor);
    while (idx !== -1) {
      if (idx > cursor) frag.appendChild(document.createTextNode(text.slice(cursor, idx)));
      const mark = document.createElement("mark");
      mark.className = "highlight";
      mark.textContent = text.slice(idx, idx + needle.length);
      frag.appendChild(mark);
      cursor = idx + needle.length;
      idx = lower.indexOf(lowerNeedle, cursor);
    }
    if (cursor < text.length) frag.appendChild(document.createTextNode(text.slice(cursor)));
    node.parentNode?.replaceChild(frag, node);
  }
}

export const MarkdownContent = memo(function MarkdownContent({ content, highlightQuery }: MarkdownContentProps) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    // Remove previous highlights
    el.querySelectorAll("mark.highlight").forEach((m) => {
      const parent = m.parentNode;
      if (!parent) return;
      parent.replaceChild(document.createTextNode(m.textContent ?? ""), m);
      parent.normalize();
    });
    const needle = highlightQuery?.trim();
    if (needle) highlightDom(el, needle);
  }, [content, highlightQuery]);

  return (
    <div className="markdown-body" ref={ref}>
      <ReactMarkdown remarkPlugins={[remarkGfm, remarkBreaks]} components={components}>
        {content}
      </ReactMarkdown>
    </div>
  );
});
