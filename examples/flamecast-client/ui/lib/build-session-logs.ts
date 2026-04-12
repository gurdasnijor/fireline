import type { ChunkRow, PermissionRow, PromptTurnRow } from "@fireline/state";
import type { SessionLog } from "../fireline-types.js";

export function buildSessionLogs(
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
              toolCallId:
                typeof toolCall.toolCallId === "string"
                  ? toolCall.toolCallId
                  : `${turn.promptTurnId}:${chunk.seq}`,
              title:
                typeof toolCall.title === "string"
                  ? toolCall.title
                  : typeof toolCall.toolName === "string"
                    ? toolCall.toolName
                    : "Tool",
              status: typeof toolCall.status === "string" ? toolCall.status : "pending",
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
              toolCallId:
                typeof toolResult.toolCallId === "string"
                  ? toolResult.toolCallId
                  : `${turn.promptTurnId}:${chunk.seq}`,
              status: typeof toolResult.status === "string" ? toolResult.status : "completed",
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
        default:
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
      type: permission.state === "pending" ? "permission_requested" : "permission_responded",
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

function parseChunkJson(content: string): Record<string, unknown> {
  try {
    const value = JSON.parse(content);
    return typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};
  } catch {
    return {};
  }
}
