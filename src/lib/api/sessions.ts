import { invoke } from "@tauri-apps/api/core";
import type { SessionDetail, SessionMessage, SessionMeta } from "@/types";

export interface DeleteSessionOptions {
  providerId: string;
  sessionId: string;
  sourcePath: string;
}

export interface ListSessionsOptions {
  scope?: "active" | "archived";
}

export interface DeleteSessionResult extends DeleteSessionOptions {
  success: boolean;
  error?: string;
}

export interface AppMetadata {
  sessions: Record<string, SessionMetadata>;
  pinnedFolders: string[];
}

interface RawAppMetadata {
  sessions?: Record<string, SessionMetadata>;
  pinnedFolders?: string[];
  pinned_folders?: string[];
}

export interface SessionMetadata {
  starred: boolean;
}

/** Fork tree types */
export interface TreeNodeData {
  sessionKey: string;
  title: string;
  summary?: string;
  lastActiveAt?: number;
  projectDir?: string;
  userHashChain: string[];
  depth: number;
  forkedAtUser: number;
  forkUserText?: string;
  children: TreeNodeData[];
}

export interface ForkTreeResult {
  roots: TreeNodeData[];
  totalSessions: number;
  computedFromCache: boolean;
  durationMs: number;
}

export interface ForkTreeOptions {
  scope?: "active" | "archived";
  projectDir?: string;
}

export const sessionsApi = {
  async list(options?: ListSessionsOptions): Promise<SessionMeta[]> {
    return await invoke("list_sessions", { options });
  },

  async getMessages(
    providerId: string,
    sourcePath: string,
  ): Promise<SessionMessage[]> {
    return await invoke("get_session_messages", { providerId, sourcePath });
  },

  async getSessionDetail(
    providerId: string,
    sourcePath: string,
  ): Promise<SessionDetail> {
    return await invoke("get_session_detail", { providerId, sourcePath });
  },

  async delete(options: DeleteSessionOptions): Promise<boolean> {
    const { providerId, sessionId, sourcePath } = options;
    return await invoke("delete_session", {
      providerId,
      sessionId,
      sourcePath,
    });
  },

  async deleteMany(
    items: DeleteSessionOptions[],
  ): Promise<DeleteSessionResult[]> {
    return await invoke("delete_sessions", { items });
  },

  async archive(options: DeleteSessionOptions): Promise<boolean> {
    const { providerId, sessionId, sourcePath } = options;
    return await invoke("archive_session", { providerId, sessionId, sourcePath });
  },

  async archiveMany(
    items: DeleteSessionOptions[],
  ): Promise<DeleteSessionResult[]> {
    return await invoke("archive_sessions", { items });
  },

  async restore(options: DeleteSessionOptions): Promise<boolean> {
    const { providerId, sessionId, sourcePath } = options;
    return await invoke("restore_session", { providerId, sessionId, sourcePath });
  },

  async restoreMany(
    items: DeleteSessionOptions[],
  ): Promise<DeleteSessionResult[]> {
    return await invoke("restore_sessions", { items });
  },

  async getAppMetadata(): Promise<AppMetadata> {
    const metadata = await invoke<RawAppMetadata>("get_app_metadata");
    return {
      sessions: metadata.sessions ?? {},
      pinnedFolders: metadata.pinnedFolders ?? metadata.pinned_folders ?? [],
    };
  },

  async setSessionStarred(sessionKey: string, starred: boolean): Promise<void> {
    return await invoke("set_session_starred", { sessionKey, starred });
  },

  async setPinnedFolders(folders: string[]): Promise<void> {
    return await invoke("set_pinned_folders", { folders });
  },

  async computeForkTree(options?: ForkTreeOptions): Promise<ForkTreeResult> {
    return await invoke("compute_fork_tree", { options });
  },

  async getForkTree(): Promise<ForkTreeResult> {
    return await invoke("get_fork_tree", {});
  },
};
