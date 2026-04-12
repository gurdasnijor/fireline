import { useQuery } from "@tanstack/react-query";
import { useFlamecastClient } from "@flamecast/ui";

/**
 * Polls the backend /health endpoint to determine connectivity.
 * Returns `isConnected: false` when the backend is unreachable.
 */
export function useBackendHealth() {
  const client = useFlamecastClient();

  const query = useQuery({
    queryKey: ["backend-health"],
    queryFn: async () => {
      const ok = await client.admin.healthCheck();
      if (!ok) {
        throw new Error("Fireline host is unreachable");
      }
      return { ok: true as const };
    },
    refetchInterval: 10_000,
    retry: 1,
    retryDelay: 2_000,
  });

  // Connected once the first successful response arrives.
  // Disconnected if the query is in error state (network failure / server down).
  const isConnected = query.isSuccess;
  const isChecking = query.isLoading;

  return { isConnected, isChecking, error: query.error };
}
