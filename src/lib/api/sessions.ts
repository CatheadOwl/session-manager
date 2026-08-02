import { invoke } from "@tauri-apps/api/core";
import type { SessionDetail, SessionLocator, SessionMessage, SessionMeta } from "@/types";
import { normalizePinnedFolders } from "@/lib/domain";

export interface SessionHandleOptions {
  providerId: string;
  sessionId: string;
  sourcePath?: string;
  locator?: SessionLocator;
}

export interface DeleteSessionOptions extends SessionHandleOptions {
  sourcePath: string;
}

const databaseRecordId = (
  locator: Extract<SessionLocator, { kind: "database" }>,
): string | undefined => locator.recordId ?? locator.record_id;

export interface ListSessionsOptions {
  scope?: "active" | "archived";
}

export interface DeleteSessionResult extends DeleteSessionOptions {
  success: boolean;
  error?: string;
}

export interface AppMetadata {
  sessions: Record<string, SessionMetadata>;
  /**
   * Pinned folder names, always in the canonical form used by folder grouping
   * (see `normalizePinnedFolders`). Read and write both normalize so persisted
   * keys from pre-separator-unification builds still match.
   */
  pinnedFolders: string[];
}

interface RawAppMetadata {
  sessions?: Record<string, SessionMetadata>;
  pinnedFolders?: string[];
  pinned_folders?: string[];
}

export const sessionHandleOptionsFromMeta = (
  session: SessionMeta,
): SessionHandleOptions => ({
  providerId: session.providerId,
  sessionId: session.sessionId,
  sourcePath: session.sourcePath,
  locator: session.locator,
});

export const loadableSessionHandleOptionsFromMeta = (
  session?: SessionMeta | null,
): SessionHandleOptions | undefined => {
  if (!session?.providerId || (!session.locator && !session.sourcePath)) {
    return undefined;
  }

  if (session.locator?.kind === "database" && !databaseRecordId(session.locator)) {
    return undefined;
  }

  return sessionHandleOptionsFromMeta(session);
};

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
    providerIdOrOptions: string | SessionHandleOptions,
    sourcePath?: string,
  ): Promise<SessionMessage[]> {
    const options =
      typeof providerIdOrOptions === "string"
        ? { providerId: providerIdOrOptions, sessionId: "", sourcePath }
        : providerIdOrOptions;
    return await invoke("get_session_messages", { ...options });
  },

  async getSessionDetail(
    providerIdOrOptions: string | SessionHandleOptions,
    sourcePath?: string,
  ): Promise<SessionDetail> {
    const options =
      typeof providerIdOrOptions === "string"
        ? { providerId: providerIdOrOptions, sessionId: "", sourcePath }
        : providerIdOrOptions;
    return await invoke("get_session_detail", { ...options });
  },

  async delete(options: DeleteSessionOptions): Promise<boolean> {
    const { providerId, sessionId, sourcePath, locator } = options;
    return await invoke("delete_session", {
      providerId,
      sessionId,
      sourcePath,
      locator,
    });
  },

  async deleteMany(
    items: DeleteSessionOptions[],
  ): Promise<DeleteSessionResult[]> {
    return await invoke("delete_sessions", { items });
  },

  async archive(options: DeleteSessionOptions): Promise<boolean> {
    const { providerId, sessionId, sourcePath, locator } = options;
    return await invoke("archive_session", { providerId, sessionId, sourcePath, locator });
  },

  async archiveMany(
    items: DeleteSessionOptions[],
  ): Promise<DeleteSessionResult[]> {
    return await invoke("archive_sessions", { items });
  },

  async restore(options: DeleteSessionOptions): Promise<boolean> {
    const { providerId, sessionId, sourcePath, locator } = options;
    return await invoke("restore_session", { providerId, sessionId, sourcePath, locator });
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
      pinnedFolders: normalizePinnedFolders(
        metadata.pinnedFolders ?? metadata.pinned_folders ?? [],
      ),
    };
  },

  async setSessionStarred(sessionKey: string, starred: boolean): Promise<void> {
    return await invoke("set_session_starred", { sessionKey, starred });
  },

  async setPinnedFolders(folders: string[]): Promise<string[]> {
    // Keep the persisted store canonical so old separator spellings never
    // re-enter storage; callers already build from normalized pins. Return the
    // normalized list so optimistic cache updates stay canonical too.
    const normalized = normalizePinnedFolders(folders);
    await invoke("set_pinned_folders", { folders: normalized });
    return normalized;
  },

  async computeForkTree(options?: ForkTreeOptions): Promise<ForkTreeResult> {
    return await invoke("compute_fork_tree", { options });
  },

  async getForkTree(): Promise<ForkTreeResult> {
    return await invoke("get_fork_tree", {});
  },
};
