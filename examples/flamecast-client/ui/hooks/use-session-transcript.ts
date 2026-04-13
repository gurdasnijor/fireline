import { useEffect, useMemo, useState } from "react";
import { useLiveQuery } from "@tanstack/react-db";
import {
  createSessionPermissionsCollection,
  createSessionTurnsCollection,
  createTurnChunksCollection,
  type ChunkRow,
} from "@fireline/state";
import { buildSessionLogs } from "../lib/build-session-logs.js";
import { useFirelineDb } from "../provider.js";

export function useSessionTranscript(sessionId: string) {
  const db = useFirelineDb();
  const [sessionChunks, setSessionChunks] = useState<ChunkRow[]>([]);
  const [chunksReady, setChunksReady] = useState(false);

  const sessionTurnsCollection = useMemo(
    () =>
      createSessionTurnsCollection({
        promptRequests: db.promptRequests,
        sessionId,
      }),
    [db, sessionId],
  );
  const sessionPermissionsCollection = useMemo(
    () =>
      createSessionPermissionsCollection({
        permissions: db.permissions,
        sessionId,
      }),
    [db, sessionId],
  );

  const turns = useLiveQuery((q) => q.from({ t: sessionTurnsCollection }), [sessionTurnsCollection]);
  const permissions = useLiveQuery(
    (q) => q.from({ p: sessionPermissionsCollection }),
    [sessionPermissionsCollection],
  );

  useEffect(() => {
    const turnRows = turns.data ?? [];
    const chunkCollections = turnRows.map((turn) =>
      createTurnChunksCollection({
        chunks: db.chunks,
        sessionId: turn.sessionId,
        requestId: turn.requestId,
      }),
    );

    const syncChunks = () => {
      const nextChunks = chunkCollections
        .flatMap((collection) => [...collection.toArray])
        .sort((left, right) => left.createdAt - right.createdAt);
      setSessionChunks(nextChunks);
      setChunksReady(true);
    };

    setChunksReady(false);
    if (chunkCollections.length === 0) {
      setSessionChunks([]);
      setChunksReady(true);
      return;
    }

    syncChunks();
    const subscriptions = chunkCollections.map((collection) =>
      collection.subscribeChanges(syncChunks),
    );

    return () => {
      for (const subscription of subscriptions) {
        subscription.unsubscribe();
      }
    };
  }, [db, turns.data]);

  const sessionTurns = useMemo(
    () => [...(turns.data ?? [])].sort((left, right) => left.startedAt - right.startedAt),
    [turns.data],
  );
  const sessionPermissions = useMemo(
    () => [...(permissions.data ?? [])].sort((left, right) => left.createdAt - right.createdAt),
    [permissions.data],
  );
  const logs = useMemo(
    () => buildSessionLogs(sessionTurns, sessionChunks, sessionPermissions),
    [sessionChunks, sessionPermissions, sessionTurns],
  );

  return {
    logs,
    isLoading: turns.isLoading || permissions.isLoading || !chunksReady,
  };
}
