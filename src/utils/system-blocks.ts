export interface SystemBlock {
  tag: string;
  content: string;
}

/**
 * Matches XML-like system metadata blocks embedded in user prompts,
 * e.g. `<system-reminder>...</system-reminder>` or `<permissions instructions>...</permissions instructions>`.
 *
 * Invariant: a harness-injected block opens with its tag at the beginning of a
 * line and closes at a line boundary — either on its own line, or at the very
 * end of the message (Codex glues some closers to the last content line, e.g.
 * a whole `<multi_agent_mode>...</multi_agent_mode>` instruction on one line).
 * A closing tag hanging mid-line is prose, not a block boundary: this both
 * skips inline pairs like `<cwd>/tmp</cwd>` and survives blocks that cite
 * themselves in their body (`...a different `<collaboration_mode>...
 * </collaboration_mode>`...`), because that citation never sits at a line
 * boundary.
 */
const SYSTEM_BLOCK_RE = /^<([\w][\w -]*)>([\s\S]*?)(?:^<\/\1>|<\/\1>\s*(?![\s\S]))/gm;

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
