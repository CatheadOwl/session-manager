import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { SessionManagerPage } from "@/components/sessions/SessionManagerPage";
import { useZoom } from "@/hooks/useZoom";

const queryClient = new QueryClient();

export default function App() {
  useZoom();

  return (
    <QueryClientProvider client={queryClient}>
      <SessionManagerPage />
    </QueryClientProvider>
  );
}
