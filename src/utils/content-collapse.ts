const ASSISTANT_PREVIEW_EDGE_LINES = 3;
const ASSISTANT_PREVIEW_MIN_LINES = 7;
const ASSISTANT_PREVIEW_MIN_LENGTH = 240;
const ASSISTANT_PREVIEW_ELLIPSIS = "...";
const LEGACY_COLLAPSE_THRESHOLD = 3000;
const LEGACY_COLLAPSED_LENGTH = 1500;

export interface CollapsedContent {
  shouldCollapse: boolean;
  previewText: string;
  fullText: string;
}

export const getAssistantPreview = (content: string): CollapsedContent => {
  const normalized = content.replace(/\r\n?/g, "\n");
  const fullText = normalized.trim();

  if (!fullText) {
    return {
      shouldCollapse: false,
      previewText: normalized,
      fullText: normalized,
    };
  }

  const lines = fullText.split("\n");
  const meaningfulLineCount = lines.filter((line) => line.trim().length > 0).length;
  const shouldCollapse = meaningfulLineCount >= ASSISTANT_PREVIEW_MIN_LINES
    || fullText.length >= ASSISTANT_PREVIEW_MIN_LENGTH;

  if (!shouldCollapse) {
    return {
      shouldCollapse: false,
      previewText: fullText,
      fullText,
    };
  }

  if (lines.length <= ASSISTANT_PREVIEW_EDGE_LINES * 2) {
    return {
      shouldCollapse: false,
      previewText: fullText,
      fullText,
    };
  }

  const firstLines = lines.slice(0, ASSISTANT_PREVIEW_EDGE_LINES);
  const lastLines = lines.slice(-ASSISTANT_PREVIEW_EDGE_LINES);
  const previewText = [
    ...firstLines,
    "",
    ASSISTANT_PREVIEW_ELLIPSIS,
    "",
    ...lastLines,
  ].join("\n");

  return {
    shouldCollapse: true,
    previewText,
    fullText,
  };
};

export const getLegacyCollapsedMessage = (content: string): CollapsedContent => {
  const shouldCollapse = content.length > LEGACY_COLLAPSE_THRESHOLD;

  if (!shouldCollapse) {
    return {
      shouldCollapse: false,
      previewText: content,
      fullText: content,
    };
  }

  return {
    shouldCollapse: true,
    previewText: `${content.slice(0, LEGACY_COLLAPSED_LENGTH)}...`,
    fullText: content,
  };
};
