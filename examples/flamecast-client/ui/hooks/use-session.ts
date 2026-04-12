import { useMemo } from "react";
import { eq } from "@tanstack/db";
import { useLiveQuery } from "@tanstack/react-db";
import { useQuery } from "@tanstack/react-query";
import type { SessionRow } from "@fireline/state";
import type { Session } from "../fireline-types.js";
import { useFirelineDb, useFlamecastClient } from "../provider.js";

export function useSession(id: string) {
  const client = useFlamecastClient();
  const db = useFirelineDb();

  const liveSession = useLiveQuery(
    (q) => q.from({ s: db.sessions }).where(({ s }) => eq(s.sessionId, id)),
    [db, id],
  );
  const metadata = useQuery({
    queryKey: ["session", id],
    queryFn: () => client.fetchSession(id),
    staleTime: Infinity,
    enabled: id.length > 0,
  });

  const data = useMemo(
    () => mergeSession(metadata.data, liveSession.data?.[0]),
    [liveSession.data, metadata.data],
  );

  return {
    ...metadata,
    data,
    isLoading: metadata.isLoading || liveSession.isLoading,
  };
}

function mergeSession(metadata: Session | undefined, row: SessionRow | undefined): Session | undefined {
  if (!metadata && !row) {
    return undefined;
  }
  if (!row) {
    return metadata;
  }

  const lastUpdatedAt = new Date(Math.max(row.updatedAt, row.lastSeenAt, row.createdAt)).toISOString();
  const status = row.state === "active" ? "active" : "killed";
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
      promptQueue: null,
      runtime: row.runtimeKey,
    } satisfies Session);

  return {
    ...base,
    status,
    lastUpdatedAt,
  };
}
