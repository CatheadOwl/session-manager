import { useQueryClient } from "@tanstack/react-query";
import { sessionsApi } from "@/lib/api/sessions";
import { queryKeys } from "@/lib/query/keys";
import type { SessionDetail, SessionMessage } from "@/types";

interface UseLatestMessageOptions {
  providerId?: string;
  sourcePath?: string;
}

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
  providerId,
  sourcePath,
}: UseLatestMessageOptions): UseLatestMessageResult {
  const queryClient = useQueryClient();

  const getLatestMessage = async (): Promise<SessionMessage | undefined> => {
    if (!providerId || !sourcePath) return;

    // Try cache first
    const cached = queryClient.getQueryData<SessionDetail>(
      queryKeys.sessionDetail(providerId, sourcePath),
    );
    if (cached?.messages?.length) {
      return [...cached.messages]
        .reverse()
        .find((m) => m.content.trim());
    }

    // Fall back to API call
    const messages = await sessionsApi.getMessages(providerId, sourcePath);
    return [...messages].reverse().find((m) => m.content.trim());
  };

  return { getLatestMessage };
}
