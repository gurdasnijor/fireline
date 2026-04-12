import { useCallback, useEffect, useMemo, useState } from "react";
import { eq } from "@tanstack/db";
import { useLiveQuery } from "@tanstack/react-db";
import { useAcpClient } from "use-acp";
import type { ChunkRow, PermissionRow, PromptTurnRow } from "@fireline/state";
import type { SessionLog, PendingPermission, PermissionResponseBody } from "../fireline-types.js";
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

  const turns = useLiveQuery(
    (q) => q.from({ t: db.collections.promptTurns }).where(({ t }) => eq(t.sessionId, sessionId)),
    [db, sessionId],
  );
  const permissions = useLiveQuery(
    (q) => q.from({ p: db.collections.permissions }).where(({ p }) => eq(p.sessionId, sessionId)),
    [db, sessionId],
  );
  const chunks = useLiveQuery((q) => q.from({ c: db.collections.chunks }), [db]);

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

  const sessionTurns = useMemo(
    () => [...(turns.data ?? [])].sort((left, right) => left.startedAt - right.startedAt),
    [turns.data],
  );
  const turnIds = useMemo(
    () => new Set(sessionTurns.map((turn) => turn.promptTurnId)),
    [sessionTurns],
  );
  const sessionChunks = useMemo(
    () =>
      [...(chunks.data ?? [])]
        .filter((chunk) => turnIds.has(chunk.promptTurnId))
        .sort((left, right) =>
          left.promptTurnId === right.promptTurnId
            ? left.seq - right.seq
            : left.createdAt - right.createdAt,
        ),
    [chunks.data, turnIds],
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
    () => sessionPermissions.filter((permission) => permission.state === "pending").map(toPendingPermission),
    [sessionPermissions],
  );
  const isProcessing = useMemo(
    () => sessionTurns.some((turn) => turn.state === "queued" || turn.state === "active"),
    [sessionTurns],
  );

  const respondToPermission = useCallback(
    (requestId: string, body: PermissionResponseBody) => {
      void client.resolvePermission(sessionId, requestId, body);
    },
    [client, sessionId],
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

  const terminate = useCallback(() => client.terminateSession(sessionId), [client, sessionId]);

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
    isLoading: sessionQuery.isLoading || turns.isLoading || permissions.isLoading || chunks.isLoading,
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

function buildSessionLogs(
  turns: PromptTurnRow[],
  chunks: ChunkRow[],
  permissions: PermissionRow[],
): SessionLog[] {
  const logs: SessionLog[] = [];
  const chunksByTurn = new Map<string, ChunkRow[]>();
  for (const chunk of chunks) {
    const current = chunksByTurn.get(chunk.promptTurnId);
    if (current) {
      current.push(chunk);
    } else {
      chunksByTurn.set(chunk.promptTurnId, [chunk]);
    }
  }

  for (const turn of turns) {
    if (turn.text) {
      logs.push({
        timestamp: new Date(turn.startedAt).toISOString(),
        type: "prompt_sent",
        data: { text: turn.text },
      });
    }

    const turnChunks = chunksByTurn.get(turn.promptTurnId) ?? [];
    for (const chunk of turnChunks) {
      switch (chunk.type) {
        case "text":
          logs.push({
            timestamp: new Date(chunk.createdAt).toISOString(),
            type: "session_update",
            data: {
              sessionUpdate: "agent_message_chunk",
              content: { type: "text", text: chunk.content },
            },
          });
          break;
        case "thinking":
          logs.push({
            timestamp: new Date(chunk.createdAt).toISOString(),
            type: "session_update",
            data: {
              sessionUpdate: "agent_thought_chunk",
              content: { type: "text", text: chunk.content },
            },
          });
          break;
        case "tool_call": {
          const toolCall = parseChunkJson(chunk.content);
          logs.push({
            timestamp: new Date(chunk.createdAt).toISOString(),
            type: "session_update",
            data: {
              sessionUpdate: "tool_call",
              toolCallId: stringField(toolCall, "toolCallId", `${turn.promptTurnId}:${chunk.seq}`),
              title: stringField(toolCall, "title", stringField(toolCall, "toolName", "Tool")),
              status: stringField(toolCall, "status", "pending"),
            },
          });
          break;
        }
        case "tool_result": {
          const toolResult = parseChunkJson(chunk.content);
          logs.push({
            timestamp: new Date(chunk.createdAt).toISOString(),
            type: "session_update",
            data: {
              sessionUpdate: "tool_call_update",
              toolCallId: stringField(toolResult, "toolCallId", `${turn.promptTurnId}:${chunk.seq}`),
              status: stringField(toolResult, "status", "completed"),
            },
          });
          break;
        }
        case "error":
          logs.push({
            timestamp: new Date(chunk.createdAt).toISOString(),
            type: "error",
            data: { message: chunk.content },
          });
          break;
        case "stop":
          logs.push({
            timestamp: new Date(chunk.createdAt).toISOString(),
            type: "prompt_completed",
            data: { promptTurnId: turn.promptTurnId, stopReason: turn.stopReason },
          });
          break;
      }
    }
  }

  for (const permission of permissions) {
    logs.push({
      timestamp: new Date(permission.createdAt).toISOString(),
      type: permissionLogType(permission),
      data: {
        requestId: permission.requestId,
        toolCallId: permission.toolCallId ?? "",
        title: permission.title ?? "Permission required",
        outcome: permission.outcome,
      },
    });
  }

  return logs.sort((left, right) => Date.parse(left.timestamp) - Date.parse(right.timestamp));
}

function permissionLogType(permission: PermissionRow): string {
  if (permission.state === "pending") {
    return "permission_requested";
  }
  switch (permission.outcome) {
    case "cancelled":
      return "permission_cancelled";
    case "rejected":
    case "deny":
      return "permission_rejected";
    case "approved":
    case "selected":
    case "allow_once":
    case "allow_always":
      return "permission_approved";
    default:
      return "permission_responded";
  }
}

function toPendingPermission(permission: PermissionRow): PendingPermission {
  return {
    requestId: permission.requestId,
    toolCallId: permission.toolCallId ?? "",
    title: permission.title ?? "Permission required",
    options:
      permission.options?.map((option) => ({
        optionId: option.optionId,
        name: option.name,
        kind: option.kind,
      })) ?? [],
  };
}

function parseChunkJson(value: string): Record<string, unknown> {
  try {
    const parsed = JSON.parse(value) as unknown;
    return isRecord(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function stringField(
  value: Record<string, unknown>,
  key: string,
  fallback: string,
): string {
  const entry = value[key];
  return typeof entry === "string" ? entry : fallback;
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
