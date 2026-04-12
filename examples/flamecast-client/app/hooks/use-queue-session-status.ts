import { useCallback, useMemo } from "react";
import { useSessions } from "@flamecast/ui";
import type { QueuedMessage } from "@/lib/message-queue-context";

export interface SessionStatus {
  processing: boolean;
  pendingPermission: boolean;
  connected: boolean;
}

/**
 * Tracks real-time processing status for sessions referenced in the message queue.
 *
 * Uses two layers:
 * 1. REST polling via useSessions() (every 5s) for baseline status
 * 2. WebSocket subscriptions per unique sessionId for real-time updates
 */
export function useQueueSessionStatus(queue: QueuedMessage[]) {
  const { data: sessions } = useSessions();

  // Stable set of unique sessionIds from the queue
  const sessionIds = useMemo(() => {
    const ids = new Set<string>();
    for (const m of queue) {
      if (m.sessionId) ids.add(m.sessionId);
    }
    return [...ids];
  }, [queue.map((m) => m.sessionId).join(",")]);

  // REST-based status from useSessions() polling
  const restStatuses = useMemo(() => {
    const map = new Map<string, SessionStatus>();
    if (!sessions) return map;
    for (const id of sessionIds) {
      const session = sessions.find((s) => s.id === id);
      if (session) {
        map.set(id, {
          processing: session.promptQueue?.processing ?? false,
          pendingPermission: !!session.pendingPermission,
          connected: true,
        });
      }
    }
    return map;
  }, [sessionIds, sessions]);

  // Merge REST and WS statuses (WS takes priority as it's more real-time)
  const getStatus = useCallback(
    (sessionId: string): SessionStatus | undefined => {
      return restStatuses.get(sessionId);
    },
    [restStatuses],
  );

  const isSessionBusy = useCallback(
    (sessionId: string): boolean => {
      const status = getStatus(sessionId);
      if (!status) return false;
      return status.processing || status.pendingPermission;
    },
    [getStatus],
  );

  return { getStatus, isSessionBusy };
}
