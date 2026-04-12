# FQA: Peer-to-peer ACP Between Two Fireline Instances

Date: 2026-04-12

Status: Completed

Scope:
- Exercise peer-to-peer ACP calls between two Fireline instances.
- Compare the exact requested `fireline run` topology against the shared-stream control-plane topology used by `examples/cross-host-discovery/`.
- Verify peer routing, B-side receipt, response streaming, state-stream reflection, and trace metadata propagation.
- Do not fix product code. Document current behavior honestly.

Build / environment:

```sh
CARGO_TARGET_DIR=/tmp/fireline-w18 cargo build --bin fireline --bin fireline-streams --bin fireline-testy
```

Notes:
- No `cargo test` was run.
- `fireline-testy` was built because the cross-host driver needs an ACP-speaking agent binary.
- Driver work was adapted from [examples/cross-host-discovery/index.ts](/Users/gnijor/gurdasnijor/fireline/examples/cross-host-discovery/index.ts).

## Overall verdict

| Scenario | Verdict | Notes |
| --- | --- | --- |
| 1. Spawn two local Fireline instances on different ports | Pass | The processes start cleanly in both tested topologies. |
| 2. A uses `peer()` to call B | Fail in exact `fireline run` topology; Pass in shared-stream topology | Separate `--streams-port` values isolate discovery. |
| 3. Verify B receives the call via ACP | Pass in shared-stream topology | Verified by raw proxied ACP envelopes and B state stream. |
| 4. Verify `_meta.traceparent` propagates A -> B | Fail | A-side initialize carries `traceparent`; B-side initialize does not. Only legacy `_meta.fireline.{traceId,parentPromptTurnId}` is forwarded. |
| 5. Verify B response streams back to A | Pass in shared-stream topology | Verified by A-side ACP response and A state-stream chunk. |
| 6. Verify both sessions' state streams reflect the interaction | Pass in shared-stream topology | A and B both materialize the peer interaction and lineage. |

## Scenario 1: Spawn two local Fireline instances on different ports

Verdict: Pass

Driver:

```sh
AGENT_BIN=/tmp/fireline-w18/debug/fireline-testy \
FIRELINE_BIN=/tmp/fireline-w18/debug/fireline \
FIRELINE_STREAMS_BIN=/tmp/fireline-w18/debug/fireline-streams \
node packages/fireline/bin/fireline.js run .tmp/fqa-peer/spec-peer.mjs \
  --port 4440 --streams-port 7474 --name agent-a --state-stream fqa-cli-a

AGENT_BIN=/tmp/fireline-w18/debug/fireline-testy \
FIRELINE_BIN=/tmp/fireline-w18/debug/fireline \
FIRELINE_STREAMS_BIN=/tmp/fireline-w18/debug/fireline-streams \
node packages/fireline/bin/fireline.js run .tmp/fqa-peer/spec-peer.mjs \
  --port 5440 --streams-port 8474 --name agent-b --state-stream fqa-cli-b
```

Observed output:

```text
agent-a ACP: ws://127.0.0.1:52412/acp
agent-a state: http://127.0.0.1:7474/v1/stream/fqa-cli-a

agent-b ACP: ws://127.0.0.1:52411/acp
agent-b state: http://127.0.0.1:8474/v1/stream/fqa-cli-b
```

Interpretation:
- The requested shape brings up two working Fireline instances.
- The key topology detail is that they do not share a durable-streams deployment stream.

## Scenario 2: A uses `peer()` to call B

Verdict: Fail in the exact requested `fireline run` topology

Driver:

```sh
cd packages/client

node_modules/.bin/tsx .tmp/fqa-peer/raw-acp-client.ts \
  --url ws://127.0.0.1:52412/acp \
  --prompt '{"command":"call_tool","server":"fireline-peer","tool":"list_peers","params":{}}' \
  --out ../../.tmp/fqa-peer/cli-list-peers.json

node_modules/.bin/tsx .tmp/fqa-peer/raw-acp-client.ts \
  --url ws://127.0.0.1:52412/acp \
  --prompt '{"command":"call_tool","server":"fireline-peer","tool":"prompt_peer","params":{"agentName":"agent-b","prompt":"{\"command\":\"echo\",\"message\":\"hello isolated streams\"}"}}' \
  --out ../../.tmp/fqa-peer/cli-prompt-peer.json
```

Observed output:

```text
list_peers response:
{"peers":[{"acpUrl":"ws://127.0.0.1:52412/acp","agentName":"agent-a","hostId":"runtime:343a467a-ffe0-4368-b34d-b04d6418f6e7","stateStreamUrl":"http://127.0.0.1:7474/v1/stream/fqa-cli-a"}]}

prompt_peer response:
ERROR: Mcp error: -32603: Internal error("peer 'agent-b' not found")
```

State-stream check:

```text
fqa-cli-a:
- prompt_turn for list_peers
- prompt_turn for prompt_peer
- chunk: ERROR: Mcp error: -32603: Internal error("peer 'agent-b' not found")
- no child_session_edge

fqa-cli-b:
- runtime_spec
- runtime_instance
- no session
- no prompt_turn
- no chunk
```

Interpretation:
- `peer()` does not discover B in the exact requested `fireline run` shape because A and B are reading different deployment streams.
- This is a real contract caveat: today `peer()` depends on shared deployment discovery, not just ACP reachability.

## Scenario 3: Verify B receives the call via ACP

Verdict: Pass in the shared-stream control-plane topology

Driver:

```sh
/tmp/fireline-w18/debug/fireline-streams
/tmp/fireline-w18/debug/fireline --control-plane --port 4440 --durable-streams-url http://127.0.0.1:7474/v1/stream
/tmp/fireline-w18/debug/fireline --control-plane --port 5440 --durable-streams-url http://127.0.0.1:7474/v1/stream

cd packages/client
AGENT_BIN=/tmp/fireline-w18/debug/fireline-testy \
OUTPUT_PATH=/Users/gnijor/gurdasnijor/fireline/.tmp/fqa-peer/shared-driver-output.json \
node_modules/.bin/tsx .tmp/fqa-peer/shared-driver.ts
```

Instrumentation note:
- The driver provisions A and B against the shared durable-streams service.
- To observe B-side ACP wire traffic, it appends a later `runtime_provisioned` discovery event that points `agent-b` at a local websocket tap on `ws://127.0.0.1:6540/acp`.
- This instrumentation changed only discovery for the test run. It did not patch product code.

Observed output:

```text
list_peers:
{"peers":[
  {"acpUrl":"ws://127.0.0.1:56140/acp","agentName":"agent-a","hostId":"runtime:4f79eef1-b17a-4f4f-92a5-d9c48fbdc31c"},
  {"acpUrl":"ws://127.0.0.1:6540/acp","agentName":"agent-b","hostId":"runtime:73a292d0-3bef-4d70-b113-5c3d047a5200"}
]}

prompt_peer response to A:
{"agentName":"agent-b","hostId":"runtime:73a292d0-3bef-4d70-b113-5c3d047a5200","responseText":"hello across shared stream","stopReason":"EndTurn"}
```

Raw ACP envelopes observed on B-side tap:

```json
{"jsonrpc":"2.0","method":"initialize","params":{"_meta":{"fireline":{"parentPromptTurnId":"fireline:agent-a:0e223790-dd0c-4b5a-96e9-7b55b0773d67:conn:12d1b97d-87a9-4264-97a1-fd2a5702be55:1","traceId":"4bf92f3577b34da6a3ce929d0e0e4736"}},"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},"protocolVersion":1},"id":"6c3c4ee5-5696-48f0-96d0-3b1c6cdc479a"}
{"jsonrpc":"2.0","method":"session/new","params":{"cwd":".","mcpServers":[]},"id":"cc5cae64-07c7-4256-9072-3518bac1c26f"}
{"jsonrpc":"2.0","method":"session/prompt","params":{"prompt":[{"text":"{\"command\":\"echo\",\"message\":\"hello across shared stream\"}","type":"text"}],"sessionId":"e93fbc24-530d-43d0-bf83-11989a4fb4ea"},"id":"ec46e37a-a797-4a6d-86f6-b9b3cd46df56"}
```

State-stream evidence on B:

```text
sessionId: e93fbc24-530d-43d0-bf83-11989a4fb4ea
parentPromptTurnId: fireline:agent-a:0e223790-dd0c-4b5a-96e9-7b55b0773d67:conn:12d1b97d-87a9-4264-97a1-fd2a5702be55:1
traceId: 4bf92f3577b34da6a3ce929d0e0e4736
prompt text: {"command":"echo","message":"hello across shared stream"}
chunk: hello across shared stream
```

Interpretation:
- In the shared-stream topology, `peer()` routes to B successfully.
- B receives the peer call over normal ACP.

## Scenario 4: Verify `_meta.traceparent` propagates A -> B

Verdict: Fail

A-side raw ACP initialize envelope:

```json
{"jsonrpc":"2.0","method":"initialize","params":{"protocolVersion":1,"clientInfo":{"name":"fqa-peer-client","version":"0.0.1"},"clientCapabilities":{"fs":{"readTextFile":false}},"_meta":{"traceparent":"00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01","fireline":{"traceId":"4bf92f3577b34da6a3ce929d0e0e4736"}}},"id":0}
```

B-side raw ACP initialize envelope, captured through the peer tap:

```json
{"jsonrpc":"2.0","method":"initialize","params":{"_meta":{"fireline":{"parentPromptTurnId":"fireline:agent-a:0e223790-dd0c-4b5a-96e9-7b55b0773d67:conn:12d1b97d-87a9-4264-97a1-fd2a5702be55:1","traceId":"4bf92f3577b34da6a3ce929d0e0e4736"}},"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},"protocolVersion":1},"id":"6c3c4ee5-5696-48f0-96d0-3b1c6cdc479a"}
```

Observed difference:
- A-side initialize includes `_meta.traceparent`.
- B-side initialize does not include `_meta.traceparent`.
- B-side initialize forwards only legacy Fireline lineage fields:
  - `_meta.fireline.traceId`
  - `_meta.fireline.parentPromptTurnId`

State-stream evidence matches the wire capture:

```text
A session/prompt_turn/child_session_edge:
- traceId = 4bf92f3577b34da6a3ce929d0e0e4736

B session/prompt_turn:
- traceId = 4bf92f3577b34da6a3ce929d0e0e4736
- parentPromptTurnId = fireline:agent-a:...:1
- no W3C traceparent field is materialized anywhere in the state stream
```

Contract drift flagged:
- The lineage handoff still uses Fireline-specific metadata instead of W3C Trace Context propagation.
- That is consistent with the current code paths in:
  - [crates/fireline-tools/src/peer/transport.rs](/Users/gnijor/gurdasnijor/fireline/crates/fireline-tools/src/peer/transport.rs:147)
  - [crates/fireline-harness/src/state_projector.rs](/Users/gnijor/gurdasnijor/fireline/crates/fireline-harness/src/state_projector.rs:676)

## Scenario 5: Verify B response streams back to A's session

Verdict: Pass in the shared-stream topology

Driver:
- Same shared-stream driver as Scenario 3.

Observed output returned to A:

```json
{"agentName":"agent-b","hostId":"runtime:73a292d0-3bef-4d70-b113-5c3d047a5200","responseText":"hello across shared stream","stopReason":"EndTurn"}
```

Settled A-side state stream:

```text
child_session_edge:
- childSessionId = e93fbc24-530d-43d0-bf83-11989a4fb4ea
- parentPromptTurnId = fireline:agent-a:0e223790-dd0c-4b5a-96e9-7b55b0773d67:conn:12d1b97d-87a9-4264-97a1-fd2a5702be55:1

A chunk:
- content = OK: CallToolResult { ... "responseText":"hello across shared stream" ... }

A prompt_turn:
- stopReason = end_turn
```

Interpretation:
- B's answer was delivered back over the parent A session as expected.

## Scenario 6: Verify both sessions' state streams reflect the interaction

Verdict: Pass in the shared-stream topology

Driver:
- Same shared-stream driver as Scenario 3.

Observed settled state on A:

```text
sessionId: b74c3562-5cf8-4654-92b5-0f00f489b21a
prompt_turn: fireline:agent-a:...:1
traceId: 4bf92f3577b34da6a3ce929d0e0e4736
child_session_edge -> childSessionId e93fbc24-530d-43d0-bf83-11989a4fb4ea
chunk -> responseText hello across shared stream
prompt_turn state -> completed, stopReason end_turn
```

Observed settled state on B:

```text
sessionId: e93fbc24-530d-43d0-bf83-11989a4fb4ea
parentPromptTurnId: fireline:agent-a:...:1
traceId: 4bf92f3577b34da6a3ce929d0e0e4736
prompt text: {"command":"echo","message":"hello across shared stream"}
chunk: hello across shared stream
prompt_turn state -> completed, stopReason end_turn
```

Interpretation:
- Both state streams reflect the same peer interaction and share the same trace lineage value.
- The exact `fireline run` topology does not do this. In that topology, B remains untouched because discovery never crosses the per-instance durable-stream boundary.

## Findings

1. `peer()` currently requires shared deployment discovery.
The exact requested `fireline run` shape with different `--streams-port` values does not produce cross-host peer routing because A and B never see the same `hosts:tenant-default` stream.

2. Peer routing works once discovery is shared.
The control-plane topology used by `examples/cross-host-discovery/` works end-to-end for `list_peers`, `prompt_peer`, B-side ACP receipt, response delivery, and state-stream lineage.

3. W3C trace context is not propagated across the peer hop.
The raw ACP envelopes show `traceparent` entering A but not leaving A for B. The peer hop preserves only legacy Fireline lineage fields.

