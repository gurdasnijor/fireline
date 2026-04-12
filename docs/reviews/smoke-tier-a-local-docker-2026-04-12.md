# Smoke Review: Tier A on Local Docker (2026-04-12)

Framing note: the same OCI image is deployable to Fly, Cloudflare Containers, or Kubernetes via target-native tooling. This review is the local-Docker validation pass that proves the substrate.

Date: 2026-04-12

Image / boot path under test:

- `5e4a707` on branch `t2-local-docker` (`docker: boot embedded specs from OCI image`)
- same code also landed on `main` as `43fee5a`

Relevant repo paths:

- `docker/fireline-host-quickstart.Dockerfile`
- `docker/bin/fireline-host-quickstart-entrypoint.sh`
- `docker/bin/fireline-embedded-spec-bootstrap.ts`
- `docker/specs/embedded-smoke-spec.json`

## Overall Verdict

Overall result: `PARTIAL PASS`

What passed:

- the embedded-spec quickstart image builds locally
- `docker run` exposes ACP on the mapped port and the endpoint is reachable
- a prompt round-trips successfully against the deployed ACP endpoint
- durable-streams data persists on an attached host volume across container restart
- an approval request survives `docker kill` / `docker start` at the durable-stream layer, and an external `approval_resolved` append still succeeds after restart

What failed:

- `docker stop` / `docker start` does **not** preserve ACP `session/load` for the earlier session
- the approval-mid-crash story does **not** complete end to end because the restarted ACP endpoint still closes on `session/load`

Important semantic caveat:

- the current approval path is the existing prompt-level fallback gate, not typed tool-call interception. The observable reason text on the permission row is `approval fallback: prompt-level gate until tool-call interception lands`.

Go / no-go:

- `GO` for demo framing that the OCI image boots locally, exposes ACP, and persists durable-streams state on mounted storage
- `NO-GO` for claiming the full unkillable-agent invariant on local Docker until restart-safe `session/load` is fixed

## Scenario Matrix

| Scenario | Result | Evidence |
| --- | --- | --- |
| Build embedded-spec OCI image | `PASS` | quickstart image built from `docker/fireline-host-quickstart.Dockerfile` with embedded spec |
| `docker run` + ACP reachability | `PASS` | `curl http://127.0.0.1:4442/healthz` returned `ok` |
| Prompt round-trip | `PASS` | ACP session returned chunk `hello docker smoke` with `stopReason: end_turn` |
| Durable-streams host volume persists | `PASS` | mounted `.tmp/fireline-embedded-spec` retained `data.log` and `meta.json` across restart |
| `docker stop` / `docker start` + `session/load` | `FAIL` | restarted ACP advertised `loadSession` but timed out on `session/load` and closed with WebSocket code `1006` |
| Approval allow path before crash | `PASS` | durable stream recorded `permission_request`, `approval_resolved`, and resumed prompt produced `Hello, world!` |
| Approval pending state survives `docker kill` | `PASS` | durable stream still contained the pending `permission_request` and active `prompt_turn` after restart |
| Grant approval after restart | `PARTIAL PASS` | external `approval_resolved` append succeeded on the durable stream after restart |
| Approval resume after restart | `FAIL` | restarted ACP still closed on `session/load`, so the paused prompt did not resume end to end |

## Reproducible Commands

### 1. Build the embedded-smoke image

```bash
docker build \
  --file docker/fireline-host-quickstart.Dockerfile \
  --build-arg SPEC=docker/specs/embedded-smoke-spec.json \
  --tag fireline-host-quickstart:embedded-smoke \
  .
```

### 2. Run the image with a durable-streams host volume

```bash
rm -rf .tmp/fireline-embedded-spec
mkdir -p .tmp/fireline-embedded-spec

docker run -d \
  --name fireline-embedded-spec-smoke \
  -p 4442:4440 \
  -p 7476:7474 \
  -v "$PWD/.tmp/fireline-embedded-spec:/var/lib/fireline" \
  fireline-host-quickstart:embedded-smoke
```

### 3. Check that ACP and durable-streams are up

```bash
curl -fsS http://127.0.0.1:4442/healthz
curl -fsS http://127.0.0.1:7476/healthz
docker logs --tail 20 fireline-embedded-spec-smoke
```

Expected signal:

- both health checks return `ok`
- container logs include `fireline: booting embedded spec 'embedded-smoke' from /etc/fireline/spec.json via existing compose()->start lowering`

### 4. Drive a prompt round-trip through ACP

```bash
node - <<'NODE' > .tmp/docker-basic-roundtrip.json
async function rpcCall(ws, id, method, params) {
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error(`timeout waiting for ${method}`)), 15000)
    const handler = (event) => {
      const msg = JSON.parse(String(event.data))
      if (msg.id === id) {
        clearTimeout(timeout)
        ws.removeEventListener('message', handler)
        resolve(msg)
      }
    }
    ws.addEventListener('message', handler)
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
}

const notifications = []
const ws = new WebSocket('ws://127.0.0.1:4442/acp')
await new Promise((resolve, reject) => {
  ws.addEventListener('open', resolve, { once: true })
  ws.addEventListener('error', reject, { once: true })
})
ws.addEventListener('message', (event) => {
  const msg = JSON.parse(String(event.data))
  if (msg.id === undefined) notifications.push(msg)
})

const initialize = await rpcCall(ws, 0, 'initialize', {
  protocolVersion: 1,
  clientInfo: { name: 'docker-smoke-client', version: '0.0.1' },
  clientCapabilities: { fs: { readTextFile: false } },
})
const sessionNew = await rpcCall(ws, 1, 'session/new', { cwd: '/workspace', mcpServers: [] })
const sessionId = sessionNew.result.sessionId
const prompt = await rpcCall(ws, 2, 'session/prompt', {
  sessionId,
  prompt: [{ type: 'text', text: '{"command":"echo","message":"hello docker smoke"}' }],
})
await new Promise((resolve) => setTimeout(resolve, 500))
ws.close()
console.log(JSON.stringify({ initialize, sessionNew, prompt, notifications }, null, 2))
NODE

sed -n '1,120p' .tmp/docker-basic-roundtrip.json
```

### 5. Restart the container and attempt `session/load`

Use the `sessionId` returned in `.tmp/docker-basic-roundtrip.json`. The example below uses the observed session id from this review.

```bash
docker stop fireline-embedded-spec-smoke
docker start fireline-embedded-spec-smoke

until curl -fsS http://127.0.0.1:4442/healthz >/dev/null; do
  sleep 1
done

node - <<'NODE' > .tmp/docker-basic-load-after-restart.json
async function rpcCall(ws, id, method, params) {
  return await new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error(`timeout waiting for ${method}`)), 15000)
    const handler = (event) => {
      const msg = JSON.parse(String(event.data))
      if (msg.id === id) {
        clearTimeout(timeout)
        ws.removeEventListener('message', handler)
        resolve(msg)
      }
    }
    ws.addEventListener('message', handler)
    ws.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  })
}

const ws = new WebSocket('ws://127.0.0.1:4442/acp')
const frames = []
let closeEvent = null
await new Promise((resolve, reject) => {
  ws.addEventListener('open', resolve, { once: true })
  ws.addEventListener('error', reject, { once: true })
})
ws.addEventListener('close', (event) => {
  closeEvent = { code: event.code, reason: event.reason }
})

frames.push(await rpcCall(ws, 0, 'initialize', {
  protocolVersion: 1,
  clientInfo: { name: 'docker-smoke-client', version: '0.0.1' },
  clientCapabilities: { fs: { readTextFile: false } },
}))

let loadResult = null
let loadError = null
try {
  loadResult = await rpcCall(ws, 1, 'session/load', {
    sessionId: '6448c292-a778-406e-8251-baf3fd8c6f5a',
  })
} catch (error) {
  loadError = String(error)
}
await new Promise((resolve) => setTimeout(resolve, 500))
ws.close()
console.log(JSON.stringify({ frames, loadResult, loadError, closeEvent }, null, 2))
NODE

sed -n '1,120p' .tmp/docker-basic-load-after-restart.json
```

### 6. Inspect the mounted durable-streams data

```bash
find .tmp/fireline-embedded-spec -maxdepth 3 -type f | sort

strings .tmp/fireline-embedded-spec/durable-streams/ZW1iZWRkZWQtc21va2Utc3RyZWFt/data.log \
  | rg 'runtime_spec|runtime_instance|6448c292-a778-406e-8251-baf3fd8c6f5a|hello docker smoke'

cat .tmp/fireline-embedded-spec/durable-streams/ZW1iZWRkZWQtc21va2Utc3RyZWFt/meta.json
```

### 7. Build an approval-gated variant

The approval proof used a temporary spec with the current prompt-level fallback approval gate:

```bash
cat > .tmp/docker-approval-spec.json <<'JSON'
{
  "kind": "harness",
  "name": "embedded-approval",
  "stateStream": "embedded-approval-stream",
  "sandbox": { "provider": "local" },
  "middleware": [
    { "kind": "approve", "scope": "tool_calls" }
  ],
  "agentCommand": ["/usr/local/bin/fireline-testy-load"]
}
JSON

docker build \
  --file docker/fireline-host-quickstart.Dockerfile \
  --build-arg SPEC=.tmp/docker-approval-spec.json \
  --tag fireline-host-quickstart:approval-smoke \
  .

rm -rf .tmp/fireline-embedded-approval
mkdir -p .tmp/fireline-embedded-approval

docker run -d \
  --name fireline-embedded-approval \
  -p 4443:4440 \
  -p 7477:7474 \
  -v "$PWD/.tmp/fireline-embedded-approval:/var/lib/fireline" \
  fireline-host-quickstart:approval-smoke
```

### 8. Approval allow path

This review used a real ACP client plus a real durable-stream append. The observed prompt text was `please pause_here for approval`.

To grant the request once its `requestId` appears on the stream:

```bash
./packages/fireline/node_modules/.bin/tsx --eval "import { appendApprovalResolved } from './packages/client/src/events.ts'; (async () => { await appendApprovalResolved({ streamUrl: 'http://127.0.0.1:7477/v1/stream/embedded-approval-stream', sessionId: 'a395015f-91de-4c6d-b156-64e311f12746', requestId: '653e89aa-ddf3-4fc9-b7bc-369661b5edf2', allow: true, resolvedBy: 'docker-smoke-manual' }); })().catch((error) => { console.error(error); process.exit(1); });"
```

### 9. Crash the approval flow mid-wait, restart, then retry resume

Observed commands:

```bash
docker kill fireline-embedded-approval
docker start fireline-embedded-approval

until curl -fsS http://127.0.0.1:4443/healthz >/dev/null; do
  sleep 1
done

strings .tmp/fireline-embedded-approval/durable-streams/ZW1iZWRkZWQtYXBwcm92YWwtc3RyZWFt/data.log \
  | rg '8fe44ff4-178c-43de-b3b5-db7977621029|31fcc755-b119-405a-9094-48f88a54804b|permission_request|approval_resolved'

./packages/fireline/node_modules/.bin/tsx --eval "import { appendApprovalResolved } from './packages/client/src/events.ts'; (async () => { await appendApprovalResolved({ streamUrl: 'http://127.0.0.1:7477/v1/stream/embedded-approval-stream', sessionId: '8fe44ff4-178c-43de-b3b5-db7977621029', requestId: '31fcc755-b119-405a-9094-48f88a54804b', allow: true, resolvedBy: 'docker-smoke-manual' }); })().catch((error) => { console.error(error); process.exit(1); });"
```

Then retry `session/load` against `ws://127.0.0.1:4443/acp` for session `8fe44ff4-178c-43de-b3b5-db7977621029`.

## Evidence

### Embedded-spec boot and ACP reachability

Observed container log:

```text
fireline: booting embedded spec 'embedded-smoke' from /etc/fireline/spec.json via existing compose()->start lowering
```

Observed health checks:

```text
$ curl -fsS http://127.0.0.1:4442/healthz
ok
$ curl -fsS http://127.0.0.1:7476/healthz
ok
```

### Prompt round-trip succeeds

From `.tmp/docker-basic-roundtrip.json`:

```json
{
  "sessionNew": {
    "result": {
      "sessionId": "6448c292-a778-406e-8251-baf3fd8c6f5a"
    }
  },
  "prompt": {
    "result": {
      "stopReason": "end_turn"
    }
  },
  "notifications": [
    {
      "method": "session/update",
      "params": {
        "update": {
          "content": {
            "text": "hello docker smoke",
            "type": "text"
          },
          "sessionUpdate": "agent_message_chunk"
        }
      }
    }
  ]
}
```

Conclusion: the embedded-spec OCI image boots, exposes ACP on the mapped host port, and serves a real prompt round-trip.

### Durable-streams state persists on the mounted host volume

Observed files after restart:

```text
.tmp/fireline-embedded-spec/durable-streams/ZW1iZWRkZWQtc21va2Utc3RyZWFt/data.log
.tmp/fireline-embedded-spec/durable-streams/ZW1iZWRkZWQtc21va2Utc3RyZWFt/meta.json
.tmp/fireline-embedded-spec/durable-streams/aG9zdHM6dGVuYW50LWRlZmF1bHQ/data.log
.tmp/fireline-embedded-spec/durable-streams/aG9zdHM6dGVuYW50LWRlZmF1bHQ/meta.json
```

Observed durable-stream excerpts:

```text
type":"runtime_spec" ... "runtime:d285abc6-0154-4713-a0a0-91a1c8249d01"
type":"runtime_instance" ... "fireline:embedded-smoke:0c033363-f7fb-4806-96d9-a472188f0c8d"
type":"prompt_turn" ... "sessionId":"6448c292-a778-406e-8251-baf3fd8c6f5a"
type":"chunk" ... "content":"hello docker smoke"
type":"runtime_spec" ... "runtime:f35e64b2-056c-491d-a434-9d5be4f70165"
type":"runtime_instance" ... "fireline:embedded-smoke:5f05be2e-d708-48f0-861a-8a72e3409c3a"
```

Observed stream metadata:

```json
{
  "name": "embedded-smoke-stream",
  "producers": {
    "state-writer-0c033363-f7fb-4806-96d9-a472188f0c8d": { "last_seq": 4 },
    "state-writer-8013682a-fefa-4d75-8865-b505debced11": { "last_seq": 4 },
    "state-writer-5f05be2e-d708-48f0-861a-8a72e3409c3a": { "last_seq": 1 }
  }
}
```

Conclusion: the mounted volume does persist durable-streams rows across container restarts. The failure is not simple disk loss.

### `session/load` after restart fails

From `.tmp/docker-basic-load-after-restart.json`:

```json
{
  "loadResult": null,
  "loadError": "Error: timeout waiting for session/load",
  "closeEvent": {
    "code": 1006,
    "reason": ""
  }
}
```

Conclusion: the restarted ACP endpoint still advertises `loadSession`, but it does not complete `session/load` for the pre-restart session.

### Approval allow path works before crash

Observed approval-allow result:

```json
{
  "pass": true,
  "sessionId": "a395015f-91de-4c6d-b156-64e311f12746",
  "requestId": "653e89aa-ddf3-4fc9-b7bc-369661b5edf2",
  "promptResponse": { "stopReason": "end_turn" },
  "updates": [
    {
      "update": {
        "content": { "text": "Hello, world!", "type": "text" }
      }
    }
  ]
}
```

Observed permission rows:

```json
{
  "kind": "permission_request",
  "sessionId": "a395015f-91de-4c6d-b156-64e311f12746",
  "requestId": "653e89aa-ddf3-4fc9-b7bc-369661b5edf2",
  "reason": "approval fallback: prompt-level gate until tool-call interception lands"
}
{
  "kind": "approval_resolved",
  "sessionId": "a395015f-91de-4c6d-b156-64e311f12746",
  "requestId": "653e89aa-ddf3-4fc9-b7bc-369661b5edf2",
  "allow": true
}
```

Conclusion: the current approval substrate works on the live Docker image, but it is the current prompt-level fallback path.

### Approval request survives `docker kill`, but resumed ACP session does not

Observed durable-stream state after killing the container mid-wait and starting it again:

```text
type":"session" ... "sessionId":"8fe44ff4-178c-43de-b3b5-db7977621029"
type":"prompt_turn" ... "text":"please pause_here during docker kill"
type":"pending_request" ... "method":"session/prompt"
type":"permission" ... "requestId":"31fcc755-b119-405a-9094-48f88a54804b"
type":"runtime_spec" ... "runtime:b527a2f1-107a-4c98-9bb2-9ab62dc56a53"
type":"runtime_instance" ... "fireline:embedded-approval:3ecfc11f-d7ab-45d5-a292-785d68c022d2"
```

Observed post-restart external approval append:

```json
{
  "kind": "approval_resolved",
  "sessionId": "8fe44ff4-178c-43de-b3b5-db7977621029",
  "requestId": "31fcc755-b119-405a-9094-48f88a54804b",
  "allow": true,
  "resolvedBy": "docker-smoke-manual"
}
```

Observed restart-resume attempt:

```json
{
  "pass": false,
  "fatalError": "ACP connection closed",
  "frames": [
    {
      "direction": "send",
      "payload": { "method": "initialize" }
    },
    {
      "direction": "recv",
      "payload": {
        "result": {
          "agentCapabilities": { "loadSession": true }
        }
      }
    },
    {
      "direction": "send",
      "payload": {
        "method": "session/load",
        "params": {
          "sessionId": "8fe44ff4-178c-43de-b3b5-db7977621029"
        }
      }
    }
  ]
}
```

Conclusion: the approval request and its later resolution survive at the durable-stream layer, but the resumed runtime still does not complete ACP `session/load`, so the full crash-and-resume invariant is not yet demo-green.

## Follow-up Notes

This review did **not** widen scope beyond the approved embedded-spec boot path. The likely next fix is in the runtime-resume path, not in Docker volume mounting:

- durable-stream rows persist across restart
- new runtime instances do register against the same state stream
- the failure is specifically the restarted ACP host not completing `session/load` for the existing session

That gap should be handled as a follow-up task, not folded into this Tier A smoke patch.
