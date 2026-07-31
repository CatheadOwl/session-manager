import { useQueryClient } from "@tanstack/react-query";
import {
  loadableSessionHandleOptionsFromMeta,
  sessionsApi,
  type SessionHandleOptions,
} from "@/lib/api/sessions";
import { queryKeys } from "@/lib/query/keys";
import type { SessionDetail, SessionMessage, SessionMeta } from "@/types";

type UseLatestMessageOptions =
  | { session: SessionMeta; providerId?: never; sourcePath?: never }
  | { session?: undefined; providerId?: string; sourcePath?: string };

interface UseLatestMessageResult {
  getLatestMessage: () => Promise<SessionMessage | undefined>;
}

/**
 * Hook that provides a function to fetch the latest non-empty message,
 * preferring the cached sessionDetail data if available.
 *
 * Used by SessionItem's "copy latest" button to avoid redundant API calls
 * when the session detail is already cached via useSessionDetailQuery.
 */
export function useLatestMessage({
  session,
  providerId,
  sourcePath,
}: UseLatestMessageOptions): UseLatestMessageResult {
  const queryClient = useQueryClient();

  const getLatestMessage = async (): Promise<SessionMessage | undefined> => {
    const handleOptions: SessionHandleOptions | undefined = session
      ? loadableSessionHandleOptionsFromMeta(session)
      : providerId && sourcePath
        ? { providerId, sessionId: "", sourcePath }
        : undefined;
    if (!handleOptions) return;

    // Try cache first
    const cached = queryClient.getQueryData<SessionDetail>(
      queryKeys.sessionDetail(
        handleOptions.providerId,
        handleOptions.locator ?? handleOptions.sourcePath,
      ),
    );
    if (cached?.messages?.length) {
      return [...cached.messages]
        .reverse()
        .find((m) => m.content.trim());
    }

    // Fall back to API call
    const messages = await sessionsApi.getMessages(handleOptions);
    return [...messages].reverse().find((m) => m.content.trim());
  };

  return { getLatestMessage };
}
