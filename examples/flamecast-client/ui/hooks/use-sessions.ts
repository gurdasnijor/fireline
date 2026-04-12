import { useMemo } from "react";
import { useLiveQuery } from "@tanstack/react-db";
import { useQuery } from "@tanstack/react-query";
import type { RequestId, SessionRow } from "@fireline/state";
import type { PendingPermission, Session } from "../fireline-types.js";
import { useFirelineDb, useFlamecastClient } from "../provider.js";

export function useSessions() {
  const client = useFlamecastClient();
  const db = useFirelineDb();

  const liveSessions = useLiveQuery((q) => q.from({ s: db.sessions }), [db]);
  const liveTurns = useLiveQuery((q) => q.from({ t: db.promptTurns }), [db]);
  const livePermissions = useLiveQuery((q) => q.from({ p: db.permissions }), [db]);
  const metadata = useQuery({
    queryKey: ["session-cache"],
    queryFn: () => client.listCachedSessions(),
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
  rows: readonly SessionRow[],
  turns: readonly { sessionId: string; state: string; text?: string | null; startedAt: number }[],
  permissions: readonly {
    sessionId: string;
    state: string;
    requestId: RequestId;
    toolCallId?: string | null;
    title?: string | null;
    options?: PendingPermission["options"];
  }[],
): Session[] {
  const metadataById = new Map(metadata.map((session) => [session.id, session]));
  const processingBySession = new Map<string, boolean>();
  const pendingPermissionBySession = new Map<string, PendingPermission>();
  const titleBySession = new Map<string, string>();

  for (const turn of turns) {
    if (turn.state === "queued" || turn.state === "active") {
      processingBySession.set(turn.sessionId, true);
    }
    if (turn.text && !titleBySession.has(turn.sessionId)) {
      titleBySession.set(turn.sessionId, turn.text);
    }
  }

  for (const permission of permissions) {
    if (permission.state === "pending") {
      pendingPermissionBySession.set(permission.sessionId, {
        requestId: permission.requestId,
        toolCallId: permission.toolCallId ?? "",
        title: permission.title ?? "Permission required",
        options: permission.options ?? [],
      });
    }
  }

  const merged: Session[] = rows.map((row) => {
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
        title: titleBySession.get(row.sessionId) ?? row.sessionId,
      } satisfies Session);

    return {
      ...base,
      status,
      lastUpdatedAt,
      title: base.title ?? titleBySession.get(row.sessionId) ?? row.sessionId,
      promptQueue: base.promptQueue
        ? {
            ...base.promptQueue,
            processing: processingBySession.get(row.sessionId) ?? false,
          }
        : {
            processing: processingBySession.get(row.sessionId) ?? false,
            paused: false,
            items: [],
            size: 0,
          },
      pendingPermission: pendingPermissionBySession.get(row.sessionId) ?? null,
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
