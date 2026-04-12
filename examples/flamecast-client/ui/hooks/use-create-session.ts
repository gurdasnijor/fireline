import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Session } from "@flamecast/sdk/session";
import { useFlamecastClient } from "../provider.js";

export function useCreateSession(options?: {
  onSuccess?: (session: Session) => void;
  onError?: (err: Error) => void;
}) {
  const client = useFlamecastClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (vars: {
      sessionId?: string;
      agentTemplateId: string;
      runtimeInstance?: string;
      cwd?: string;
      /** Display name for optimistic sidebar entry. */
      agentName?: string;
    }) =>
      client.createSession({
        sessionId: vars.sessionId,
        agentTemplateId: vars.agentTemplateId,
        cwd: vars.cwd,
        runtimeInstance: vars.runtimeInstance,
      }),
    onMutate: async (vars) => {
      if (!vars.sessionId) return;
      await queryClient.cancelQueries({ queryKey: ["session-cache"] });
      const previous = queryClient.getQueryData<Session[]>(["session-cache"]);
      const placeholder: Session = {
        id: vars.sessionId,
        agentName: vars.agentName ?? "Starting...",
        spawn: { command: "", args: [] },
        startedAt: new Date().toISOString(),
        lastUpdatedAt: new Date().toISOString(),
        status: "active",
        logs: [],
        pendingPermission: null,
        fileSystem: null,
        promptQueue: null,
        runtime: vars.runtimeInstance,
        cwd: vars.cwd,
      };
      client.rememberSession(placeholder);
      queryClient.setQueryData<Session[]>(["session-cache"], (old) => {
        const existing = old?.filter((session) => session.id !== placeholder.id) ?? [];
        return [placeholder, ...existing];
      });
      queryClient.setQueryData(["session-cache", vars.sessionId], placeholder);
      return { previous, placeholderId: vars.sessionId };
    },
    onSuccess: (session) => {
      client.rememberSession(session);
      queryClient.setQueryData<Session[]>(["session-cache"], (old) => {
        const existing = old?.filter((entry) => entry.id !== session.id) ?? [];
        return [session, ...existing];
      });
      queryClient.setQueryData(["session-cache", session.id], session);
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
      options?.onSuccess?.(session);
    },
    onError: (err, _vars, context) => {
      if (context?.placeholderId) {
        client.forgetSession(context.placeholderId);
        queryClient.removeQueries({ queryKey: ["session-cache", context.placeholderId], exact: true });
      }
      if (context?.previous) {
        queryClient.setQueryData(["session-cache"], context.previous);
      }
      options?.onError?.(err);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ["session-cache"] });
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
    },
  });
}
