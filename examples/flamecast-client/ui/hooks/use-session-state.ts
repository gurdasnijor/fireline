import { useCallback, useEffect, useMemo, useState } from "react";
import { useLiveQuery } from "@tanstack/react-db";
import { useAcpClient } from "use-acp";
import {
  type ChunkRow,
  createSessionPermissionsCollection,
  createSessionTurnsCollection,
  createTurnChunksCollection,
} from "@fireline/state";
import type { PendingPermission, PermissionResponseBody } from "../fireline-types.js";
import { buildSessionLogs } from "../lib/build-session-logs.js";
import { useFirelineDb, useFlamecastClient } from "../provider.js";
import { sessionLogsToSegments } from "../lib/logs-markdown.js";
import { useSession } from "./use-session.js";
import type { RuntimeWebSocketHandle } from "./use-runtime-websocket.js";

export function useSessionState(sessionId: string, _ws: RuntimeWebSocketHandle) {
  const client = useFlamecastClient();
  const db = useFirelineDb();
  const sessionQuery = useSession(sessionId);
  const session = sessionQuery.data;
  const [showAllFiles, setShowAllFiles] = useState(false);
  const [workspaceRoot, setWorkspaceRoot] = useState<string | null>(session?.fileSystem?.root ?? null);
  const [sessionChunks, setSessionChunks] = useState<ChunkRow[]>([]);
  const [chunksReady, setChunksReady] = useState(false);

  const sessionTurnsCollection = useMemo(
    () =>
      createSessionTurnsCollection({
        promptTurns: db.promptTurns,
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

  const acp = useAcpClient({
    wsUrl: session?.websocketUrl ?? "",
    autoConnect: !!session?.websocketUrl,
    initialSessionId: sessionId,
    sessionParams: {
      cwd: session?.cwd ?? "/workspace",
      mcpServers: [],
    },
  });

  useEffect(() => {
    if (session?.fileSystem?.root) {
      setWorkspaceRoot(session.fileSystem.root);
    }
  }, [session?.fileSystem?.root]);

  useEffect(() => {
    const turnRows = turns.data ?? [];
    const chunkCollections = turnRows.map((turn) =>
      createTurnChunksCollection({
        chunks: db.chunks,
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
    [sessionTurns, sessionChunks, sessionPermissions],
  );
  const markdownSegments = useMemo(() => sessionLogsToSegments(logs), [logs]);
  const pendingPermissions = useMemo(
    () => (acp.pendingPermission?.sessionId === sessionId ? [toPendingPermissionRequest(acp.pendingPermission)] : []),
    [acp.pendingPermission, sessionId],
  );
  const isProcessing = useMemo(
    () => sessionTurns.some((turn) => turn.state === "queued" || turn.state === "active"),
    [sessionTurns],
  );

  const respondToPermission = useCallback(
    (requestId: string, body: PermissionResponseBody) => {
      if (acp.pendingPermission?.deferredId !== requestId) {
        throw new Error("Permission is no longer active on the ACP connection");
      }
      acp.resolvePermission({
        outcome:
          "optionId" in body
            ? { outcome: "selected", optionId: body.optionId }
            : { outcome: "cancelled" },
      });
    },
    [acp],
  );

  const prompt = useCallback(
    (text: string) => {
      if (!acp.agent) {
        throw new Error("ACP agent is not connected yet");
      }
      return acp.agent.prompt({
        sessionId,
        prompt: [{ type: "text", text }],
      });
    },
    [acp.agent, sessionId],
  );

  const cancel = useCallback(() => {
    if (!acp.agent) {
      return Promise.resolve();
    }
    return acp.agent.cancel({ sessionId });
  }, [acp.agent, sessionId]);

  const terminate = useCallback(async () => {
    if (!session?.sandboxId) {
      throw new Error("Session sandbox is not available yet");
    }
    await client.admin.destroy(session.sandboxId);
    client.forgetSession(sessionId);
  }, [client, session?.sandboxId, sessionId]);

  const requestFsSnapshot = useCallback(
    async (opts?: { showAllFiles?: boolean }) => {
      const snapshot = await client.fetchSessionFileSystem(sessionId, {
        showAllFiles: opts?.showAllFiles,
      });
      setWorkspaceRoot(snapshot.root);
      return snapshot;
    },
    [client, sessionId],
  );

  const requestFilePreview = useCallback(
    async (path: string) => client.fetchSessionFilePreview(sessionId, path),
    [client, sessionId],
  );

  const connectionState = mapConnectionState(acp.connectionState.status);

  return {
    session,
    isLoading: sessionQuery.isLoading || turns.isLoading || permissions.isLoading || !chunksReady,
    connectionState,
    isConnected: connectionState === "connected",
    logs,
    markdownSegments,
    isProcessing,
    pendingPermissions,
    respondToPermission,
    workspaceRoot,
    showAllFiles,
    setShowAllFiles,
    prompt,
    cancel,
    terminate,
    requestFilePreview,
    requestFsSnapshot,
  };
}

function toPendingPermissionRequest(permission: NonNullable<ReturnType<typeof useAcpClient>["pendingPermission"]>): PendingPermission {
  return {
    requestId: permission.deferredId,
    toolCallId: permission.toolCall.toolCallId ?? "",
    title: permission.toolCall.title ?? "Permission required",
    kind: permission.toolCall.kind ?? undefined,
    options: permission.options.map((option) => ({
        optionId: option.optionId,
        name: option.name,
        kind: option.kind,
      })),
  };
}

function mapConnectionState(
  status: string,
): "disconnected" | "connecting" | "connected" | "reconnecting" {
  switch (status) {
    case "connected":
      return "connected";
    case "connecting":
      return "connecting";
    case "reconnecting":
      return "reconnecting";
    default:
      return "disconnected";
  }
}
