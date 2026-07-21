export interface SystemBlock {
  tag: string;
  content: string;
}

/**
 * Matches XML-like system metadata blocks embedded in user prompts,
 * e.g. `<system-reminder>...</system-reminder>` or `<permissions instructions>...</permissions instructions>`.
 * Only matches blocks whose opening tag starts at the beginning of a line.
 */
const SYSTEM_BLOCK_RE = /^<([\w][\w -]*)>([\s\S]*?)^<\/\1>/gm;

/**
 * Extracts system metadata blocks from raw message text.
 * Returns the remaining text (with blocks removed) and the extracted blocks.
 */
export function extractSystemBlocks(text: string): { text: string; blocks: SystemBlock[] } {
  const blocks: SystemBlock[] = [];
  const cleaned = text.replace(SYSTEM_BLOCK_RE, (_match, tag: string, content: string) => {
    blocks.push({ tag: tag.trim(), content: content.trim() });
    return "";
  });
  return { text: cleaned.trim(), blocks };
}
