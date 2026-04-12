import { useMemo } from "react";
import { useLiveQuery } from "@tanstack/react-db";
import { useQuery } from "@tanstack/react-query";
import type { SessionRow } from "@fireline/state";
import type { Session } from "../fireline-types.js";
import { useFirelineDb, useFlamecastClient } from "../provider.js";

export function useSessions() {
  const client = useFlamecastClient();
  const db = useFirelineDb();

  const liveSessions = useLiveQuery((q) => q.from({ s: db.sessions }), [db]);
  const liveTurns = useLiveQuery((q) => q.from({ t: db.promptTurns }), [db]);
  const livePermissions = useLiveQuery((q) => q.from({ p: db.permissions }), [db]);
  const metadata = useQuery({
    queryKey: ["sessions"],
    queryFn: () => client.fetchSessions(),
    staleTime: Infinity,
  });

  const data = useMemo(
    () =>
      mergeSessions(
        metadata.data ?? [],
        liveSessions.data ?? [],
        liveTurns.data ?? [],
        livePermissions.data ?? [],
      ),
    [livePermissions.data, liveSessions.data, liveTurns.data, metadata.data],
  );

  return {
    ...metadata,
    data,
    isLoading:
      metadata.isLoading ||
      liveSessions.isLoading ||
      liveTurns.isLoading ||
      livePermissions.isLoading,
  };
}

function mergeSessions(
  metadata: Session[],
  rows: SessionRow[],
  turns: readonly { sessionId: string; state: string }[],
  permissions: readonly { sessionId: string; state: string }[],
): Session[] {
  const metadataById = new Map(metadata.map((session) => [session.id, session]));
  const processingBySession = new Map<string, boolean>();
  const pendingPermissionBySession = new Map<string, boolean>();

  for (const turn of turns) {
    if (turn.state === "queued" || turn.state === "active") {
      processingBySession.set(turn.sessionId, true);
    }
  }

  for (const permission of permissions) {
    if (permission.state === "pending") {
      pendingPermissionBySession.set(permission.sessionId, true);
    }
  }

  const merged = rows.map((row) => {
    const current = metadataById.get(row.sessionId);
    const lastUpdatedAt = new Date(Math.max(row.updatedAt, row.lastSeenAt, row.createdAt)).toISOString();
    const status = toSessionStatus(row.state);
    const base: Session =
      current ??
      ({
        id: row.sessionId,
        agentName: row.sessionId,
        spawn: { command: "", args: [] },
        startedAt: new Date(row.createdAt).toISOString(),
        lastUpdatedAt,
        status,
        logs: [],
        pendingPermission: null,
        fileSystem: null,
        promptQueue: null,
        runtime: row.runtimeKey,
      } satisfies Session);

    return {
      ...base,
      status,
      lastUpdatedAt,
      promptQueue: base.promptQueue
        ? {
            ...base.promptQueue,
            processing: processingBySession.get(row.sessionId) ?? false,
          }
        : null,
      pendingPermission:
        pendingPermissionBySession.get(row.sessionId) ?? false
          ? base.pendingPermission
          : null,
    };
  });

  const rowIds = new Set(rows.map((row) => row.sessionId));
  for (const session of metadata) {
    if (!rowIds.has(session.id)) {
      merged.push(session);
    }
  }

  return merged.sort(
    (left, right) => Date.parse(right.lastUpdatedAt) - Date.parse(left.lastUpdatedAt),
  );
}

function toSessionStatus(state: SessionRow["state"]): Session["status"] {
  return state === "active" ? "active" : "killed";
}
