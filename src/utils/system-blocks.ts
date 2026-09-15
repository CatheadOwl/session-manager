export interface SystemBlock {
  tag: string;
  content: string;
}

/**
 * Matches XML-like system metadata blocks embedded in user prompts,
 * e.g. `<system-reminder>...</system-reminder>` or `<permissions instructions>...</permissions instructions>`.
 * Only matches blocks whose opening tag starts at the beginning of a line.
 *
 * Codex harness blocks (`collaboration_mode` / `multi_agent_role` / `multi_agent_mode`)
 * glue the closing tag to the last content line instead of putting it on its own
 * line, so they match greedily to the LAST closing tag — the body may cite the
 * tag itself (`...a different `<collaboration_mode>...</collaboration_mode>`...`),
 * and a non-greedy match would stop at that mention.
 */
const SYSTEM_BLOCK_RE =
  /^<([\w][\w -]*)>([\s\S]*?)^<\/\1>|^<(collaboration_mode|multi_agent_role|multi_agent_mode)>([\s\S]*)<\/\3>/gm;

/**
 * Extracts system metadata blocks from raw message text.
 * Returns the remaining text (with blocks removed) and the extracted blocks.
 */
export function extractSystemBlocks(text: string): { text: string; blocks: SystemBlock[] } {
  const blocks: SystemBlock[] = [];
  const cleaned = text.replace(
    SYSTEM_BLOCK_RE,
    (
      _match,
      genericTag: string | undefined,
      genericContent: string | undefined,
      inlineTag: string | undefined,
      inlineContent: string | undefined,
    ) => {
      blocks.push({
        tag: (genericTag ?? inlineTag ?? "").trim(),
        content: (genericContent ?? inlineContent ?? "").trim(),
      });
      return "";
    },
  );
  return { text: cleaned.trim(), blocks };
}
