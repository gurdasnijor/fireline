import { useMemo } from "react";
import { eq } from "@tanstack/db";
import { useLiveQuery } from "@tanstack/react-db";
import { useQuery } from "@tanstack/react-query";
import type { RequestId, SessionRow } from "@fireline/state";
import type { PendingPermission, Session } from "../fireline-types.js";
import { useFirelineDb, useFlamecastClient } from "../provider.js";

export function useSession(id: string) {
  const client = useFlamecastClient();
  const db = useFirelineDb();

  const liveSession = useLiveQuery(
    (q) => q.from({ s: db.sessions }).where(({ s }) => eq(s.sessionId, id)),
    [db, id],
  );
  const liveTurns = useLiveQuery(
    (q) => q.from({ t: db.promptTurns }).where(({ t }) => eq(t.sessionId, id)),
    [db, id],
  );
  const livePermissions = useLiveQuery(
    (q) => q.from({ p: db.permissions }).where(({ p }) => eq(p.sessionId, id)),
    [db, id],
  );
  const metadata = useQuery({
    queryKey: ["session-cache", id],
    queryFn: () => client.getCachedSession(id) ?? null,
    staleTime: Infinity,
    enabled: id.length > 0,
  });

  const data = useMemo(
    () => mergeSession(metadata.data ?? undefined, liveSession.data?.[0], liveTurns.data ?? [], livePermissions.data ?? []),
    [livePermissions.data, liveSession.data, liveTurns.data, metadata.data],
  );

  return {
    ...metadata,
    data,
    isLoading: metadata.isLoading || liveSession.isLoading || liveTurns.isLoading || livePermissions.isLoading,
  };
}

function mergeSession(
  metadata: Session | undefined,
  row: SessionRow | undefined,
  turns: readonly { text?: string | null; startedAt: number }[],
  permissions: readonly {
    state: string;
    requestId: RequestId;
    toolCallId?: string | null;
    title?: string | null;
    options?: PendingPermission["options"];
  }[],
): Session | undefined {
  if (!metadata && !row) {
    return undefined;
  }
  if (!row) {
    return metadata;
  }

  const lastUpdatedAt = new Date(Math.max(row.updatedAt, row.lastSeenAt, row.createdAt)).toISOString();
  const status = row.state === "active" ? "active" : "killed";
  const title = turns.find((turn) => turn.text)?.text ?? metadata?.title ?? row.sessionId;
  const pendingPermission = permissions.find((permission) => permission.state === "pending");
  const base: Session =
    metadata ??
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
      promptQueue: {
        processing: false,
        paused: false,
        items: [],
        size: 0,
      },
      runtime: row.runtimeKey,
      title,
    } satisfies Session);

  return {
    ...base,
    status,
    lastUpdatedAt,
    title,
    pendingPermission: pendingPermission
      ? {
          requestId: pendingPermission.requestId,
          toolCallId: pendingPermission.toolCallId ?? "",
          title: pendingPermission.title ?? "Permission required",
          options: pendingPermission.options ?? [],
        }
      : null,
  };
}
