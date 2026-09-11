export interface SessionMeta {
  providerId: string;
  sessionId: string;
  title?: string;
  summary?: string;
  projectDir?: string | null;
  createdAt?: number;
  lastActiveAt?: number;
  sourcePath?: string;
  locator?: SessionLocator;
  resumeCommand?: string;
}

export type SessionLocator =
  | { kind: "file"; path: string }
  | { kind: "database"; path: string; recordId?: string; record_id?: string }
  /** Read-only SSH remote session; sourceId anchors to the ssh source entry's required `id`. */
  | { kind: "remote"; sourceId: string; path: string };

export interface TokenUsage {
  inputTokens: number;
  cacheCreationInputTokens: number;
  cacheReadInputTokens: number;
  outputTokens: number;
}

export interface CumulativeTokenUsage {
  inputTokens: number;
  outputTokens: number;
  totalTokens: number;
}

export interface ToolCallInfo {
  name: string;
  input: string;
  callId?: string;
}

export interface ToolResultInfo {
  content: string;
  callId?: string;
}

export interface SessionMessage {
  role: string;
  content: string;
  ts?: number;
  usage?: TokenUsage;
  cumulativeUsage?: CumulativeTokenUsage;
  toolCalls?: ToolCallInfo[];
  toolResult?: ToolResultInfo;
}

export interface QaPair {
  questionIdx: number;
  answerIdx: number;
}

export interface SessionDetail {
  messages: SessionMessage[];
  qaPairs: QaPair[];
  rawContent?: string | null;
}

// ─── Q&A export (time-ranged, session-level, provenance-preserving) ─────────

export interface QaEntry {
  question: string;
  answer: string;
  ts?: number;
}

export interface SessionProvenance {
  providerId: string;
  sessionId: string;
  title?: string;
  projectDir?: string | null;
  createdAt?: number;
  lastActiveAt?: number;
  locator?: SessionLocator;
}

export interface QaSessionExport {
  provenance: SessionProvenance;
  qa: QaEntry[];
}

export interface ExportSkippedItem {
  providerId: string;
  sessionId: string;
  error: string;
}

export interface ExportOutcome {
  count: number;
  skipped: ExportSkippedItem[];
  destPath: string;
}
