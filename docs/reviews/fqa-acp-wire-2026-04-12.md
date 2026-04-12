# ACP Wire Conformance QA

Date: 2026-04-12

Scope: raw ACP over the running `fireline-host` WebSocket interface at `/acp`, using real binaries built into `CARGO_TARGET_DIR=/tmp/fireline-w15`.

Artifacts used:
- Driver: `/tmp/fireline_acp_fqa_2026_04_12.mjs`
- Raw run report: `/tmp/fireline-acp-fqa-artifacts/report.json`
- Binaries:
  - `/tmp/fireline-w15/debug/fireline`
  - `/tmp/fireline-w15/debug/fireline-streams`
  - `/tmp/fireline-w15/debug/fireline-testy`
  - `/tmp/fireline-w15/debug/fireline-testy-load`

Build/run method:
- `CARGO_TARGET_DIR=/tmp/fireline-w15 cargo build --bin fireline --bin fireline-streams --bin fireline-testy --bin fireline-testy-load`
- Durable streams sidecar: `PORT=50374 /tmp/fireline-w15/debug/fireline-streams`
- Host binary: `/tmp/fireline-w15/debug/fireline ... --durable-streams-url http://127.0.0.1:50374/v1/stream -- <agent>`

## Summary

| Scenario | Verdict | Notes |
| --- | --- | --- |
| 1. `session.new` | Pass | `SessionId` returned as plain ACP string UUID, no Fireline prefixing on wire |
| 2. `session.prompt` simple text | Pass | `session/update` envelope shape matched ACP schema |
| 3. approval policy / resume | Fail | prompt resumed, but no ACP `requestPermission` appeared; approval is off-wire via durable-stream events |
| 4. timeout error | Pass | returned valid JSON-RPC error envelope; message is Fireline-specific |
| 5. concurrent sessions | Pass | two `SessionId`s multiplexed over one ACP connection, no observed cross-talk |
| 6. `session.load` after crash | Fail | `session/load` returned `session_not_found` even though the durable stream already contained the session row and transcript |

## Driver

All six scenarios used the same raw Node driver:
- open a native `WebSocket` to `ws://127.0.0.1:<port>/acp`
- send literal JSON-RPC objects for `initialize`, `session/new`, `session/prompt`, `session/load`
- record every outbound and inbound frame verbatim
- for approval scenarios only, append `approval_resolved` directly to the durable stream via HTTP `POST /v1/stream/<name>`

No SDK client wrapper was used for the ACP traffic itself.

## 1. `session.new`

Driver steps:
1. `initialize`
2. `session/new`

Observed ACP:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{"fs":{"readTextFile":false}},"clientInfo":{"name":"fqa-driver","version":"0.0.1"}}}
{"jsonrpc":"2.0","result":{"agentCapabilities":{"loadSession":true,"mcpCapabilities":{"http":false,"sse":false},"promptCapabilities":{"audio":false,"embeddedContext":false,"image":false},"sessionCapabilities":{}},"authMethods":[],"protocolVersion":1},"id":1}
{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/private/tmp/fireline-fqa-acp-xUBxoY","mcpServers":[]}}
{"jsonrpc":"2.0","result":{"sessionId":"572c31c1-fbd9-4cdc-9572-cf09c4636975"},"id":2}
```

Checks:
- `protocolVersion: 1` round-tripped cleanly
- capabilities were present in ACP shape
- `sessionId` came back as an opaque ACP string, not a Fireline-decorated identifier

Verdict: Pass.

## 2. `session.prompt` with simple text

Driver steps:
1. `initialize`
2. `session/new`
3. `session/prompt`

Observed ACP:

```json
{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"bc0a3078-6971-4519-850d-d574fe547d09","prompt":[{"type":"text","text":"hello from raw acp"}]}}
{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"bc0a3078-6971-4519-850d-d574fe547d09","update":{"content":{"text":"Hello, world!","type":"text"},"sessionUpdate":"agent_message_chunk"}}}
{"jsonrpc":"2.0","result":{"stopReason":"end_turn"},"id":3}
```

Checks:
- notification method was `session/update`
- params used `sessionId`
- update payload used ACP discriminant `sessionUpdate: "agent_message_chunk"`
- prompt response used `stopReason: "end_turn"`

Verdict: Pass.

## 3. approval policy, resolve, resume

Host config:
- topology included `approval_gate`
- policy matched prompt substring `pause_here`

Driver steps:
1. `initialize`
2. `session/new`
3. `session/prompt` with `please pause_here for approval`
4. poll durable stream for `permission_request`
5. append `approval_resolved`
6. await prompt completion

Observed ACP:

```json
{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"2284406e-d02e-42ee-8288-1e8f19cdbe25","prompt":[{"type":"text","text":"please pause_here for approval"}]}}
{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"2284406e-d02e-42ee-8288-1e8f19cdbe25","update":{"content":{"text":"Hello, world!","type":"text"},"sessionUpdate":"agent_message_chunk"}}}
{"jsonrpc":"2.0","result":{"stopReason":"end_turn"},"id":3}
```

Observed durable-stream side evidence:

```json
{
  "type": "permission",
  "key": "2284406e-d02e-42ee-8288-1e8f19cdbe25:02462cc2-41db-4bd4-8564-46b6e4846c5d",
  "value": {
    "kind": "permission_request",
    "sessionId": "2284406e-d02e-42ee-8288-1e8f19cdbe25",
    "requestId": "02462cc2-41db-4bd4-8564-46b6e4846c5d",
    "reason": "fqa policy"
  }
}
```

What matched:
- prompt did block until an external `approval_resolved` append
- prompt did resume cleanly after resolution

What drifted:
- no ACP `requestPermission` request appeared on the wire at all
- approval flow is implemented off-wire via durable-stream `permission_request` / `approval_resolved`
- the approval `requestId` was not the raw JSON-RPC request id from the ACP prompt frame (`id: 3`); Fireline emitted a different synthetic UUID into the state stream

Verdict: Fail.

Reason: the scenario requirement was ACP approval behavior. Fireline currently suspends/resumes via durable-stream side effects instead of an ACP-native permission exchange.

## 4. timeout path

Host config:
- same `approval_gate` topology
- `timeoutMs: 500`

Observed ACP:

```json
{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"46ef9e89-c8b2-447e-a3e4-19aee8892b89","prompt":[{"type":"text","text":"please pause_here until timeout"}]}}
{"jsonrpc":"2.0","error":{"code":-32603,"message":"Internal error","data":"approval_gate timed out waiting for approval on session 46ef9e89-c8b2-447e-a3e4-19aee8892b89"},"id":3}
```

Checks:
- timeout surfaced as a valid JSON-RPC error envelope
- request id matched the original prompt request
- error data was present and readable

Note:
- error typing is generic (`-32603 Internal error`), not a more specific ACP/domain code

Verdict: Pass.

## 5. concurrent sessions

Method:
- one ACP connection
- two `session/new`
- two `session/prompt` requests sent before either response was awaited

Observed ACP:

```json
{"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":"2e815974-a244-4e45-88aa-b690ba8b1c81","prompt":[{"type":"text","text":"echo A"}]}}
{"jsonrpc":"2.0","id":5,"method":"session/prompt","params":{"sessionId":"b22792e4-5a5d-48eb-b363-a501a25e6118","prompt":[{"type":"text","text":"echo B"}]}}
{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"2e815974-a244-4e45-88aa-b690ba8b1c81","update":{"content":{"text":"Hello, world!","type":"text"},"sessionUpdate":"agent_message_chunk"}}}
{"jsonrpc":"2.0","result":{"stopReason":"end_turn"},"id":4}
{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"b22792e4-5a5d-48eb-b363-a501a25e6118","update":{"content":{"text":"Hello, world!","type":"text"},"sessionUpdate":"agent_message_chunk"}}}
{"jsonrpc":"2.0","result":{"stopReason":"end_turn"},"id":5}
```

Checks:
- two distinct `SessionId`s were created
- `session/update` notifications stayed scoped to the matching `sessionId`
- no update for session A was labeled as session B, or vice versa

Verdict: Pass.

## 6. `session.load` after crash

Method:
1. start host A with `fireline-testy-load`
2. `initialize`
3. `session/new`
4. `session/prompt`
5. wait until the session row is visible in the durable stream
6. hard-stop host A
7. start host B against the same durable stream and same state stream name
8. `initialize`
9. `session/load`

Observed pre-crash durable state:

```json
{
  "type": "session",
  "key": "0c06268c-035f-43d8-bfe4-63569c34f1d1",
  "value": {
    "sessionId": "0c06268c-035f-43d8-bfe4-63569c34f1d1",
    "state": "active",
    "supportsLoadSession": true
  }
}
```

```json
{
  "type": "chunk",
  "value": {
    "sessionId": "0c06268c-035f-43d8-bfe4-63569c34f1d1",
    "content": "Hello, world!"
  }
}
```

Observed ACP after restart:

```json
{"jsonrpc":"2.0","id":2,"method":"session/load","params":{"sessionId":"0c06268c-035f-43d8-bfe4-63569c34f1d1","cwd":"/private/tmp/fireline-fqa-acp-xUBxoY","mcpServers":[]}}
{"jsonrpc":"2.0","error":{"code":-32061,"message":"session_not_found","data":{"sessionId":"0c06268c-035f-43d8-bfe4-63569c34f1d1"}},"id":2}
```

Checks:
- the session record and transcript chunk were already durable before the crash
- the replacement host still rejected `session/load`

Verdict: Fail.

Reason: the running host did not honor `session/load` against a session that was already present in the durable stream.

## ACP / schema drift observed

Wire-level drift:
- approval gating is not ACP-native today; no `requestPermission` was emitted in scenario 3
- timeout path uses generic JSON-RPC `-32603 Internal error` rather than a more specific domain error shape

State-adjacent drift discovered while validating the wire:
- approval correlation uses a stream-side `requestId` that is not the raw ACP JSON-RPC `id`
- agent-plane stream rows still contain synthetic Fireline ids and infrastructure leakage, including:
  - `promptTurnId`
  - `traceId`
  - `logicalConnectionId`
  - `chunkId`
  - `seq`
  - `runtimeId`
  - `runtimeKey`
  - `nodeId`

Representative pre-crash row from scenario 6:

```json
{
  "type": "prompt_turn",
  "value": {
    "promptTurnId": "fireline:scenario6-load-before-crash:ab28b5d2-20ca-43b6-9eed-e62cd09a3281:conn:4915f2b6-fbd5-4d05-afd2-f22d0993f860:1",
    "requestId": "48390f98-9fda-433b-957a-ef943e027e5d",
    "sessionId": "0c06268c-035f-43d8-bfe4-63569c34f1d1",
    "traceId": "fireline:scenario6-load-before-crash:ab28b5d2-20ca-43b6-9eed-e62cd09a3281:conn:4915f2b6-fbd5-4d05-afd2-f22d0993f860:1"
  }
}
```

## Overall conclusion

`fireline-host` is wire-conformant for the basic ACP session lifecycle:
- `initialize`
- `session/new`
- `session/prompt`
- `session/update`

The two substantive gaps from this run are:
- approval policy handling is still a Fireline-specific durable-stream side channel, not ACP permission flow
- `session/load` after host crash is not working against durable state that is already present on the stream

Those are the highest-priority ACP functional QA failures from the 2026-04-12 run.
