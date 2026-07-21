export interface SessionMeta {
  providerId: string;
  sessionId: string;
  title?: string;
  summary?: string;
  projectDir?: string | null;
  createdAt?: number;
  lastActiveAt?: number;
  sourcePath?: string;
  resumeCommand?: string;
}

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
