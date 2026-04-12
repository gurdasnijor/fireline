import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Session } from "../fireline-types.js";
import { useFlamecastClient } from "../provider.js";

export function useTerminateSession(options?: {
  onSuccess?: (id: string) => void;
  onError?: (err: Error) => void;
}) {
  const client = useFlamecastClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (id: string) => {
      const session = client.getCachedSession(id);
      if (!session?.sandboxId) {
        throw new Error("Session sandbox is not available yet");
      }
      await client.admin.destroy(session.sandboxId);
      client.forgetSession(id);
    },
    onSuccess: (_result, id) => {
      queryClient.setQueryData(["session-cache", id], null);
      queryClient.setQueryData(["session-cache"], (current: Session[] | undefined) =>
        current?.filter((session) => session.id !== id) ?? [],
      );
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
      options?.onSuccess?.(id);
    },
    onError: options?.onError,
  });
}
