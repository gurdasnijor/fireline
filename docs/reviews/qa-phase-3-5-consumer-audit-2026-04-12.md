# Phase 3.5 Chunk Payload Consumer Audit — 2026-04-12

Scope: post-`714cc84` scan of `packages/client/`, `packages/state/`, and `examples/` for residual coupling to the pre-Phase-3.5 chunk row shape (`ChunkType`, `chunk_type`, `chunk.type`, `chunk.content`).

Primary grep:

```bash
rg -n "chunk_type|chunk\\.type|chunk\\.content|ChunkType" packages/client/ packages/state/ examples/
```

Result: no remaining production-code matches. One test-only leftover bug remains, documented below.

## Summary

- Production consumers are migrated to `chunk.update: SessionUpdate`.
- `examples/flamecast-client/`, `examples/multi-agent-team/`, `examples/cross-host-discovery/`, and `examples/live-monitoring/` no longer depend on the removed `type/content` pair.
- `packages/state` now provides the compatibility helpers that current consumers actually use: `extractChunkTextPreview(update)` and `isToolCallSessionUpdate(update)`.
- One test still asserts the deleted chunk row shape: `packages/client/test/acp.test.ts`.

## Helper Check — `extractChunkTextPreview`

Source: [packages/state/src/session-updates.ts](../../packages/state/src/session-updates.ts)

Current behavior:

- `user_message_chunk`, `agent_message_chunk`, `agent_thought_chunk` → returns `content.text`
- `tool_call` → returns title/tool name
- `tool_call_update` → returns status
- everything else → returns `''`

Verdict:

- `ToolCall` / `ToolCallUpdate`: preserved well enough for current preview consumers.
- `Plan` / other non-text `SessionUpdate` variants: not preserved as preview text; they are dropped.
- Current consumers do not appear to rely on plan-like variants for display or control flow, so this is a limitation, not a confirmed break.

## Per-Consumer Audit

### `packages/state`

| Consumer | Verdict | Notes |
|---|---|---|
| `packages/state/src/collections/turn-chunks.ts` | Pass | Collection filters by `requestId` and returns `ChunkRow` with `update`; no old `type/content` coupling. |
| `packages/state/src/session-updates.ts` | Pass with limitation | New helper layer is canonical. Preview helper intentionally collapses only text/tool-call variants. |
| `packages/state/test/collections.test.ts` | Pass | Test coverage already exercises `extractChunkTextPreview(row.update)` on canonical `SessionUpdate` rows. |
| `packages/state/test/flamecast-build-session-logs.test.ts` | Pass | Explicitly validates `tool_call` + `tool_call_update` preservation through Flamecast log building. |

### `packages/client`

| Consumer | Verdict | Notes |
|---|---|---|
| `packages/client` runtime code | Pass | No production package client code still references `ChunkType`, `chunk.type`, or `chunk.content`. |
| `packages/client/test/acp.test.ts` | Fail | Still asserts removed fields: `entry.promptTurnId`, `entry.content`, and `entry.type` at [packages/client/test/acp.test.ts](../../packages/client/test/acp.test.ts). This is a real post-Phase-3.5 regression in test coverage. |
| `packages/client/test/topology.test.ts` | Pass | Reads ACP `SessionUpdate` payloads directly from the ACP connection, not from the chunk row schema. |

### `examples/multi-agent-team`

| Consumer | Verdict | Notes |
|---|---|---|
| [examples/multi-agent-team/index.ts](../../examples/multi-agent-team/index.ts) | Pass | Reconstructs text with `extractChunkTextPreview(row.update)`. No direct chunk-shape coupling remains. |

### `examples/cross-host-discovery`

| Consumer | Verdict | Notes |
|---|---|---|
| [examples/cross-host-discovery/index.ts](../../examples/cross-host-discovery/index.ts) | Pass | Same pattern as multi-agent-team: reads `row.update` through `extractChunkTextPreview`. |

### `examples/live-monitoring`

| Consumer | Verdict | Notes |
|---|---|---|
| [examples/live-monitoring/index.ts](../../examples/live-monitoring/index.ts) | Pass | Counts tool calls through `isToolCallSessionUpdate(row.update)`. No old shape references remain. |

### `examples/flamecast-client`

| Consumer | Verdict | Notes |
|---|---|---|
| [examples/flamecast-client/server.ts](../../examples/flamecast-client/server.ts) | Pass | Server gathers `ChunkRow[]` and delegates rendering to `buildSessionLogs(...)`; no old `type/content` assumptions at the server layer. |
| [examples/flamecast-client/ui/lib/build-session-logs.ts](../../examples/flamecast-client/ui/lib/build-session-logs.ts) | Pass | Emits raw `chunk.update` into session logs and keys only on `sessionUpdateKind(...)`. |
| [examples/flamecast-client/ui/lib/logs-markdown.ts](../../examples/flamecast-client/ui/lib/logs-markdown.ts) | Pass | Pattern-matches canonical `SessionUpdate` payloads (`agent_message_chunk`, `user_message_chunk`, `tool_call`, `tool_call_update`) rather than removed chunk row fields. |
| [examples/flamecast-client/ui/hooks/use-session-state.ts](../../examples/flamecast-client/ui/hooks/use-session-state.ts) | Pass | Builds transcript state from `createTurnChunksCollection(...)` + `buildSessionLogs(...)`; no legacy chunk shape assumptions. |
| [examples/flamecast-client/ui/hooks/use-session-transcript.ts](../../examples/flamecast-client/ui/hooks/use-session-transcript.ts) | Pass | Same as above. |

## Bug List

### Bug 1 — `packages/client/test/acp.test.ts` still expects removed chunk fields

Status: open as `mono-7t6`

Evidence:

- [packages/client/test/acp.test.ts](../../packages/client/test/acp.test.ts) still searches `db.collections.chunks` with:
  - `entry.promptTurnId === promptTurn?.promptTurnId`
  - `entry.content.includes('Hello')`
  - `expect(chunk?.type).toBe('text')`

Why this is a bug:

- `ChunkRow` no longer exposes `promptTurnId`, `content`, or `type`.
- The canonical row now exposes `sessionId`, `requestId`, `toolCallId?`, and `update: SessionUpdate`.

Required fix:

- Retype the assertion to match `chunk_v2`, likely by joining on `requestId` and asserting against `chunk.update`.

Beads:

- `mono-7t6` created as `P1 bug`
- dependency added: `mono-7t6` blocks `mono-vkpp.6`

## Conclusion

Phase 3.5 did not leave behind production consumer coupling to `ChunkType` / `content:String`. The remaining risk is narrow and concrete: one stale `packages/client` test still encodes the deleted chunk row shape. The preview helper also does not surface plan-like `SessionUpdate` variants, but no current consumer in `packages/` or `examples/` depends on that behavior today.
