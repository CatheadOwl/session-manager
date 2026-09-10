import { useEffect } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { listen } from "@tauri-apps/api/event";
import { SessionManagerPage } from "@/components/sessions/SessionManagerPage";
import { queryKeys } from "@/lib/query/keys";
import { useZoom } from "@/hooks/useZoom";

const queryClient = new QueryClient();

/**
 * Settings core (ADR 0006): the Rust side emits one `settings-changed` event
 * (payload `{ keys }`) after any programmatic write; we listen once at the
 * app root and invalidate the settings cache so every consumer refetches.
 */
function useSettingsChangedListener(client: QueryClient) {
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    listen<{ keys: string[] }>("settings-changed", (event) => {
      if (event.payload.keys.length > 0) {
        client.invalidateQueries({ queryKey: queryKeys.settings() });
      }
    }).then((un) => {
      if (cancelled) {
        un();
      } else {
        unlisten = un;
      }
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [client]);
}

export default function App() {
  useZoom();
  useSettingsChangedListener(queryClient);

  return (
    <QueryClientProvider client={queryClient}>
      <SessionManagerPage />
    </QueryClientProvider>
  );
}
