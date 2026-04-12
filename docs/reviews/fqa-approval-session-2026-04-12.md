# FQA Review: Approval-Gated Session Round Trip

Date: 2026-04-12

Commit under test: `6bdb3a278b2638147b30cef1dc79c27b0d632552`

## Scope

This review drives the approval-gated session flow as a real user against real local binaries and real endpoints. Per the dispatch, this is not `cargo test`; the evidence comes from:

- `npx tsx packages/fireline/bin/fireline.js run ...`
- real ACP websocket sessions
- real durable-stream SSE/state reads
- real `appendApprovalResolved(...)` appends
- a real `kill -9` of the runtime host PID

## Overall Verdict

Overall result: `FAIL`

What works:

- `fireline run` boots a live harness and prints usable ACP/state URLs
- a raw ACP client can connect, create a session, prompt, observe `permission_request`, append `approval_resolved`, and receive the resumed result
- denied approvals block execution and return an ACP error instead of executing the write

What does not meet the requested demo bar:

- the live approval gate is still prompt-gated fallback, not true tool-call interception
- after a real runtime-host `kill -9`, re-provisioning the same spec against the same state stream does not round-trip through `session/load`; the new ACP endpoint closes the connection immediately after the `session/load` request

## Driver Scripts Used

### 1. Approval FS harness spec

```ts
// .tmp/fqa/approval-fs-spec.ts
export default {
  kind: 'harness',
  name: 'fqa-approval-fs',
  async start(options) {
    return await fetch(`${options.serverUrl}/v1/sandboxes`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        name: options.name ?? 'fqa-approval-fs',
        agentCommand: ['/tmp/fireline-w19/debug/fireline-testy-fs'],
        provider: 'local',
        stateStream: options.stateStream,
        resources: [],
        topology: {
          components: [
            {
              name: 'approval_gate',
              config: {
                timeoutMs: 60000,
                policies: [
                  {
                    match: { kind: 'promptContains', needle: '' },
                    action: 'requireApproval',
                    reason:
                      'FQA prompt-level approval fallback while tool-call interception is not wired',
                  },
                ],
              },
            },
            {
              name: 'fs_backend',
              config: { backend: 'runtime_stream' },
            },
          ],
        },
      }),
    }).then((r) => r.json())
  },
}
```

### 2. ACP + approval driver

```ts
// packages/client/.tmp/fqa/approval-driver.ts
const permissionStream = new DurableStream({
  url: options.stateUrl,
  contentType: 'application/json',
})
const response = await permissionStream.stream({ live: 'sse', json: true })
response.subscribeJson((batch) => {
  for (const item of batch.items) {
    if (item.type === 'permission') permissionEvents.push(item)
  }
})

await acp.connection.initialize({ protocolVersion: PROTOCOL_VERSION, ... })
const session = await acp.connection.newSession({ cwd: '/workspace', mcpServers: [] })
const permission = await waitForPermission(permissionEvents, session.sessionId, options.timeoutMs)

await appendApprovalResolved({
  streamUrl: options.stateUrl,
  sessionId: session.sessionId,
  requestId: permission.requestId,
  allow: options.approve,
  resolvedBy: 'fqa-approval-driver',
})

const response = await acp.connection.prompt({
  sessionId: session.sessionId,
  prompt: [{ type: 'text', text: defaultPrompt(options.scenario) }],
})
```

### 3. Restart helper

```ts
// .tmp/fqa/start-spec.ts
const mod = await import(pathToFileURL(resolve(specPath)).href)
const spec = mod.default
const handle = await spec.start({ serverUrl, stateStream, name })
console.log(JSON.stringify(handle, null, 2))
```

## Scenario Matrix

| Scenario | Result | Evidence |
| --- | --- | --- |
| 1. Boot local Fireline via CLI | `PASS` | printed ACP/state URLs from live `fireline run` |
| 2. Open ACP session programmatically | `PASS` | raw JSON-RPC `initialize` + `session/new` frames |
| 3. Trigger tool call behind approval policy | `PARTIAL / FAIL AS WRITTEN` | live gate fired on prompt, not on tool call |
| 4. Observe `permission_request` on SSE state stream | `PASS` | raw durable-stream `permission` envelopes captured |
| 5. Resolve via `appendApprovalResolved(..., true)` | `PASS` | `approval_resolved` envelope appended with matching request id |
| 6. Allow path resumes and returns result | `PASS` | fs write produced `ok:/workspace/fqa-approved.txt`; echo produced `alpha after approval` |
| 7. Kill runtime host, restart, `session/load` transcript round trip | `FAIL` | new ACP accepted `initialize`, then closed on `session/load` |
| 8. Denied approval blocks execution | `PASS` with UX caveat | no fs success chunk; ACP returned generic `Internal error` |

## Live Commands

### FS-backed allow/deny run

```bash
FIRELINE_BIN=/tmp/fireline-w19/debug/fireline \
FIRELINE_STREAMS_BIN=/tmp/fireline-w19/debug/fireline-streams \
npx tsx packages/fireline/bin/fireline.js run .tmp/fqa/approval-fs-spec.ts \
  --port 4540 \
  --streams-port 7574 \
  --state-stream fqa-approval-fs-stream
```

Observed boot output:

```text
✓ fireline ready

  sandbox:   runtime:d7f04581-711b-4780-bae4-ac99a82addff
  ACP:       ws://127.0.0.1:50721/acp
  state:     http://127.0.0.1:7574/v1/stream/fqa-approval-fs-stream
```

### Resumable-agent run

```bash
FIRELINE_BIN=/tmp/fireline-w19/debug/fireline \
FIRELINE_STREAMS_BIN=/tmp/fireline-w19/debug/fireline-streams \
npx tsx packages/fireline/bin/fireline.js run .tmp/fqa/approval-load-spec.ts \
  --port 4541 \
  --streams-port 7575 \
  --state-stream fqa-approval-load-stream
```

Observed boot output:

```text
✓ fireline ready

  sandbox:   runtime:c639d69e-6d51-4e70-b834-52aa9344125d
  ACP:       ws://127.0.0.1:54793/acp
  state:     http://127.0.0.1:7575/v1/stream/fqa-approval-load-stream
```

## Scenario Evidence

### 1. Allow path: approval request observed, resolved, prompt resumes

Driver command:

```bash
./packages/fireline/node_modules/.bin/tsx packages/client/.tmp/fqa/approval-driver.ts \
  --scenario fs-allow \
  --acp-url ws://127.0.0.1:50721/acp \
  --state-url http://127.0.0.1:7574/v1/stream/fqa-approval-fs-stream \
  --approve true \
  --timeout-ms 20000
```

Observed result:

- `pass: true`
- `sessionId: 6501b4b7-943f-4c2d-884b-a6fcd1f16464`
- `requestId: e9575908-1248-435a-99bf-5fc18be298c6`
- ACP update chunk: `ok:/workspace/fqa-approved.txt`

Raw permission envelopes:

```json
{
  "type": "permission",
  "value": {
    "kind": "permission_request",
    "sessionId": "6501b4b7-943f-4c2d-884b-a6fcd1f16464",
    "requestId": "e9575908-1248-435a-99bf-5fc18be298c6"
  }
}
{
  "type": "permission",
  "value": {
    "kind": "approval_resolved",
    "sessionId": "6501b4b7-943f-4c2d-884b-a6fcd1f16464",
    "requestId": "e9575908-1248-435a-99bf-5fc18be298c6",
    "allow": true
  }
}
{
  "type": "chunk",
  "value": {
    "sessionId": "6501b4b7-943f-4c2d-884b-a6fcd1f16464",
    "content": "ok:/workspace/fqa-approved.txt"
  }
}
```

Conclusion: the live stack proves `permission_request -> approval_resolved -> resumed execution -> ACP result`.

### 2. Denied path: write blocked, ACP returns error, no fs success chunk

Driver command:

```bash
./packages/fireline/node_modules/.bin/tsx packages/client/.tmp/fqa/approval-driver.ts \
  --scenario fs-deny \
  --acp-url ws://127.0.0.1:50721/acp \
  --state-url http://127.0.0.1:7574/v1/stream/fqa-approval-fs-stream \
  --approve false \
  --timeout-ms 20000
```

Observed result:

- `pass: true`
- `sessionId: a4f30761-a875-497a-bf59-25e1c13bb4b1`
- `requestId: 8a77e54f-16b2-44a0-998d-89505a15e6a2`
- `promptError: "Internal error"`
- prompt turn ended as `state: "broken"` with `stopReason: "error"`
- no success chunk for `/workspace/fqa-denied.txt`

Raw ACP error:

```json
{
  "jsonrpc": "2.0",
  "error": {
    "code": -32603,
    "message": "Internal error",
    "data": "approval_gate denied by approver: FQA prompt-level approval fallback while tool-call interception is not wired"
  },
  "id": 2
}
```

Raw prompt-turn terminal state:

```json
{
  "type": "prompt_turn",
  "value": {
    "sessionId": "a4f30761-a875-497a-bf59-25e1c13bb4b1",
    "state": "broken",
    "stopReason": "error"
  }
}
```

Conclusion: denied approval blocks the write correctly, but the user-facing ACP error is still the generic `Internal error`.

### 3. Approval is still prompt-level fallback, not tool-call interception

This is not speculation; the live run only became possible by wiring policies against `promptContains`, and the emitted reason string matched the fallback wording:

```json
{
  "kind": "permission_request",
  "reason": "FQA prompt-level approval fallback while tool-call interception is not wired"
}
```

The codebase agrees with that live behavior:

- `packages/client/src/sandbox.ts` lowers `approve({ scope: 'tool_calls' })` through a prompt-level fallback path
- `crates/fireline-harness/src/approval.rs` documents that the current live gate intercepts `session/prompt`

Conclusion: scenario 3 only passes in the weaker sense of "the prompt that eventually causes the tool action is approval-gated". Literal tool-call interception is not yet present.

### 4. Load-capable allow path works before the crash

Driver command:

```bash
./packages/fireline/node_modules/.bin/tsx packages/client/.tmp/fqa/approval-driver.ts \
  --scenario load-allow \
  --acp-url ws://127.0.0.1:54793/acp \
  --state-url http://127.0.0.1:7575/v1/stream/fqa-approval-load-stream \
  --approve true \
  --timeout-ms 20000
```

Observed result:

- `pass: true`
- `sessionId: 54baf42b-4615-4c58-b1c1-81d9a04f7cca`
- `requestId: 3ae98b6b-513f-4bdd-89f8-044848443a60`
- ACP chunk: `alpha after approval`

Key ACP frames:

```json
{ "direction": "send", "payload": { "method": "session/prompt", "id": 2 } }
{
  "direction": "recv",
  "payload": {
    "method": "session/update",
    "params": {
      "sessionId": "54baf42b-4615-4c58-b1c1-81d9a04f7cca",
      "update": { "sessionUpdate": "agent_message_chunk", "content": { "text": "alpha after approval" } }
    }
  }
}
{ "direction": "recv", "payload": { "id": 2, "result": { "stopReason": "end_turn" } } }
```

Conclusion: before the crash, the resumable agent can still round-trip through approval and produce output.

### 5. Crash + restart + `session/load` fails

Crash command:

```bash
kill -9 34750
```

`34750` was the live runtime host PID serving the original ACP URL `ws://127.0.0.1:54793/acp`.

Observed immediately after kill:

- ACP port `54793` stopped listening
- control plane at `http://127.0.0.1:4541/healthz` stayed healthy
- control plane still reported the dead runtime as `status:"ready"`

Stale old-runtime descriptor after kill:

```json
{
  "id": "runtime:c639d69e-6d51-4e70-b834-52aa9344125d",
  "status": "ready",
  "acp": { "url": "ws://127.0.0.1:54793/acp" }
}
```

Restart command:

```bash
./packages/fireline/node_modules/.bin/tsx .tmp/fqa/start-spec.ts \
  .tmp/fqa/approval-load-spec.ts \
  http://127.0.0.1:4541 \
  fqa-approval-load-stream \
  fqa-approval-load
```

Restarted runtime handle:

```json
{
  "id": "runtime:c5e0e05b-320c-4ce6-ba29-74947082839b",
  "acp": { "url": "ws://127.0.0.1:55851/acp" },
  "state": { "url": "http://127.0.0.1:7575/v1/stream/fqa-approval-load-stream" }
}
```

Reconnect/load driver command:

```bash
./packages/fireline/node_modules/.bin/tsx packages/client/.tmp/fqa/approval-driver.ts \
  --scenario load-resume \
  --acp-url ws://127.0.0.1:55851/acp \
  --state-url http://127.0.0.1:7575/v1/stream/fqa-approval-load-stream \
  --session-id 54baf42b-4615-4c58-b1c1-81d9a04f7cca \
  --timeout-ms 20000
```

Observed result:

- `pass: false`
- `fatalError: "ACP connection closed"`
- `initialize` succeeded
- `session/load` was sent
- no `session/load` response arrived
- the durable stream only showed the new `connection` envelope, not a successful resumed prompt

Captured ACP frames:

```json
{ "direction": "send", "payload": { "method": "initialize", "id": 0 } }
{ "direction": "recv", "payload": { "id": 0, "result": { "agentCapabilities": { "loadSession": true } } } }
{
  "direction": "send",
  "payload": {
    "method": "session/load",
    "id": 1,
    "params": { "sessionId": "54baf42b-4615-4c58-b1c1-81d9a04f7cca" }
  }
}
```

No matching `recv` frame followed. The SDK surfaced:

```text
ACP connection closed
```

Conclusion: the live crash/restart/session-load story is not demo-ready. The transcript remains on the durable stream, but the re-provisioned runtime does not complete the `session/load` round trip.

## Secondary Findings

### 1. Control-plane liveness drift after hard kill

After `kill -9` of the runtime host PID, `GET /v1/sandboxes/<old-id>` still returned `status:"ready"` with the dead ACP URL. The control plane did not mark the runtime dead.

### 2. Permission projection drift in `@fireline/state`

The first driver attempt could not rely on `db.collections.permissions` because the projected row shape did not match the raw stream envelope. In the captured projection, `requestId` was rewritten to the durable-stream key:

```json
{
  "kind": "permission_request",
  "requestId": "54baf42b-4615-4c58-b1c1-81d9a04f7cca:3ae98b6b-513f-4bdd-89f8-044848443a60"
}
```

while the raw envelope carried:

```json
{
  "kind": "permission_request",
  "requestId": "3ae98b6b-513f-4bdd-89f8-044848443a60"
}
```

For live approval observers, the raw stream is currently the trustworthy source.

## Follow-Up Dispatch Candidates

1. Replace the prompt-level approval fallback with real tool-call interception so scenario 3 is literal, not approximate.
2. Fix runtime re-provision + `session/load` after host death; current behavior is ACP disconnect after sending `session/load`.
3. Fix control-plane liveness/status after hard runtime-host death.
4. Align `@fireline/state` permission projection with the raw `permission_request` / `approval_resolved` envelope shape.

## Bottom Line

The live approval flow works today only as a prompt-level approval gate. The allow and deny paths both function end to end with real binaries and real durable-stream events. The demo-blocking gap is crash/restart: after a real host kill, a fresh runtime on the same state stream does not survive `session/load`, and the connection closes before the resumed transcript can be observed.
