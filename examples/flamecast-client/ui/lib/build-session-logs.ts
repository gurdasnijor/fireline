import {
  sessionUpdateKind,
  type ChunkRow,
  type PermissionRow,
  type PromptTurnRow,
} from "@fireline/state";
import type { SessionLog } from "../fireline-types.js";

export function buildSessionLogs(
  turns: PromptTurnRow[],
  chunks: ChunkRow[],
  permissions: PermissionRow[],
): SessionLog[] {
  const logs: SessionLog[] = [];
  const chunksByRequest = new Map<string | number | null, ChunkRow[]>();
  for (const chunk of chunks) {
    const current = chunksByRequest.get(chunk.requestId);
    if (current) {
      current.push(chunk);
    } else {
      chunksByRequest.set(chunk.requestId, [chunk]);
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

    const turnChunks = chunksByRequest.get(turn.requestId) ?? [];
    for (const chunk of turnChunks) {
      if (sessionUpdateKind(chunk.update)) {
        logs.push({
          timestamp: new Date(chunk.createdAt).toISOString(),
          type: "session_update",
          data: chunk.update as Record<string, unknown>,
        });
      }
    }

    if (
      turn.state === "completed" ||
      turn.state === "broken" ||
      turn.state === "cancelled" ||
      turn.state === "timed_out"
    ) {
      logs.push({
        timestamp: new Date(turn.completedAt ?? turn.startedAt).toISOString(),
        type: "prompt_completed",
        data: { requestId: turn.requestId, stopReason: turn.stopReason },
      });
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
