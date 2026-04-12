# Peer-to-Peer Demo Capture

Status: replayable demo capture for the peer-fleet slice

Source of truth:
- [docs/reviews/fqa-peer-to-peer-2026-04-12.md](./../reviews/fqa-peer-to-peer-2026-04-12.md)
- [docs/demos/pi-acp-to-openclaw.md](./pi-acp-to-openclaw.md) Step 4
- [docs/demos/pi-acp-to-openclaw-operator-script.md](./pi-acp-to-openclaw-operator-script.md) Step 6

## Prerequisites

- `fireline`, `fireline-streams`, and `fireline-testy` are already built locally.
  The replay script does not run `cargo`.
- `node` and `curl` are installed.
- `packages/client/dist/` exists in the repo checkout. The replay driver imports the built client package directly from there.
- Ports `7474`, `4440`, and `5440` are free, or the operator overrides `STREAMS_PORT`, `CONTROL_A_PORT`, and `CONTROL_B_PORT`.
- The demo uses the **shared durable-streams topology** from FQA-5, not two isolated `fireline run` invocations with different `--streams-port` values.

Recommended env:

```bash
export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
export AGENT_BIN="$PWD/target/debug/fireline-testy"
```

## Honest scope

Pass path used for the replay:
- shared `fireline-streams`
- two `fireline --control-plane` instances
- `agent-a` discovers and prompts `agent-b` via `peer()`
- both state streams reflect the same interaction

Known caveats carried forward from FQA-5:
- The exact `fireline run` topology with different `--streams-port` values does **not** discover peers across hosts today.
- `_meta.traceparent` does **not** propagate across the peer hop today. Only Fireline lineage fields (`traceId`, `parentPromptTurnId`) survive.
- The raw B-side ACP tap used in FQA-5 was test instrumentation, not a product surface. This capture doc uses normal stdout plus state-stream evidence.

## Quick Replay

One-shot replay:

1. `FIRELINE_BIN="$PWD/target/debug/fireline" FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams" AGENT_BIN="$PWD/target/debug/fireline-testy" ./docs/demos/scripts/replay-peer-to-peer.sh`

Expected stdout excerpt:

```json
{
  "summaryPath": "/.../.tmp/peer-to-peer-demo/latest/summary.json",
  "listPeersExcerpt": "...\"agentName\":\"agent-a\"...\"agentName\":\"agent-b\"...",
  "promptPeerExcerpt": "...\"responseText\":\"hello across shared stream\"...",
  "agentAStateStream": "http://127.0.0.1:7474/v1/stream/peer-demo-a",
  "agentBStateStream": "http://127.0.0.1:7474/v1/stream/peer-demo-b",
  "note": "traceparent forwarding is a known failure and is not captured by this script"
}
```

Artifacts written by the script:
- `.tmp/peer-to-peer-demo/latest/summary.json`
- `.tmp/peer-to-peer-demo/latest/logs/fireline-streams.log`
- `.tmp/peer-to-peer-demo/latest/logs/control-plane-a.log`
- `.tmp/peer-to-peer-demo/latest/logs/control-plane-b.log`

## Manual Replay

This is the same path, broken out into operator-visible commands. Each numbered step is one command.

1. `export FIRELINE_BIN="$PWD/target/debug/fireline"`

Expected result:
- no stdout

2. `export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"`

Expected result:
- no stdout

3. `export AGENT_BIN="$PWD/target/debug/fireline-testy"`

Expected result:
- no stdout

4. `./docs/demos/scripts/replay-peer-to-peer.sh setup-only`

Expected stdout excerpt:

```text
[peer-demo] starting fireline-streams
[peer-demo] starting control-plane-a
[peer-demo] starting control-plane-b
```

Expected side effect:
- `.tmp/peer-to-peer-demo/latest/logs/` now contains three process logs

5. `curl -fsS http://127.0.0.1:7474/healthz && curl -fsS http://127.0.0.1:4440/healthz && curl -fsS http://127.0.0.1:5440/healthz`

Expected stdout:

```text
okokok
```

6. `./docs/demos/scripts/replay-peer-to-peer.sh driver-only`

Expected stdout excerpt:

```json
{
  "summaryPath": "/.../.tmp/peer-to-peer-demo/latest/summary.json",
  "listPeersExcerpt": "...\"agentName\":\"agent-a\"...\"agentName\":\"agent-b\"...",
  "promptPeerExcerpt": "...\"responseText\":\"hello across shared stream\"...",
  "agentAStateStream": "http://127.0.0.1:7474/v1/stream/peer-demo-a",
  "agentBStateStream": "http://127.0.0.1:7474/v1/stream/peer-demo-b"
}
```

Pass marker:
- `listPeersExcerpt` contains both `agent-a` and `agent-b`
- `promptPeerExcerpt` contains `hello across shared stream`

7. `cat .tmp/peer-to-peer-demo/latest/summary.json`

Expected JSON excerpts:

```json
{
  "topology": {
    "sharedDiscoveryRequired": true
  },
  "interaction": {
    "promptMessage": "hello across shared stream"
  },
  "state": {
    "agentA": {
      "childSessionEdges": [
        {
          "childSessionId": "...",
          "parentPromptTurnId": "fireline:agent-a:..."
        }
      ]
    },
    "agentB": {
      "promptTurns": [
        {
          "parentPromptTurnId": "fireline:agent-a:...",
          "traceId": "..."
        }
      ],
      "chunks": [
        {
          "content": "hello across shared stream"
        }
      ]
    }
  },
  "limitations": {
    "isolatedFirelineRunStreamsFailDiscovery": true,
    "traceparentForwardedAcrossPeerHop": false,
    "rawBsideAcpTapCapturedByThisScript": false
  }
}
```

Pass marker:
- `agentA.childSessionEdges` is non-empty
- `agentB.chunks` contains `hello across shared stream`
- `traceparentForwardedAcrossPeerHop` is `false` and is documented as a current gap, not a demo failure

8. `./docs/demos/scripts/replay-peer-to-peer.sh teardown-only`

Expected stdout excerpt:

```text
[peer-demo] stopping control-plane-b (...)
[peer-demo] stopping control-plane-a (...)
[peer-demo] stopping fireline-streams (...)
```

## Expected Outputs By Surface

### Operator stdout

Expected success markers:
- health checks return `ok`
- `driver-only` prints a `summaryPath`
- the prompt-peer excerpt includes `responseText":"hello across shared stream"`

### ACP transcript excerpts

This replay does not attach the FQA-5 websocket tap by default. The operator should use these excerpts from the validated QA run as the expected wire shape:

```json
{"jsonrpc":"2.0","method":"initialize","params":{"_meta":{"fireline":{"parentPromptTurnId":"fireline:agent-a:...","traceId":"4bf92f3577b34da6a3ce929d0e0e4736"}},"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false},"protocolVersion":1},"id":"..."}
{"jsonrpc":"2.0","method":"session/prompt","params":{"prompt":[{"text":"{\"command\":\"echo\",\"message\":\"hello across shared stream\"}","type":"text"}],"sessionId":"..."},"id":"..."}
```

Honest caveat:
- `_meta.traceparent` is absent on the B-side initialize envelope today. That is a known fail from FQA-5, not operator error.

### Stream contents

Expected A-side evidence:
- one prompt turn for the peer call
- one `child_session_edge` linking A's prompt turn to B's child session
- one chunk containing the tool result with `responseText":"hello across shared stream"`

Expected B-side evidence:
- one prompt turn whose `parentPromptTurnId` points back to A
- one chunk with `hello across shared stream`
- matching `traceId` with A's interaction

## Pass / Fail Markers

| Step | Expected verdict | What counts as pass | What counts as fail |
| --- | --- | --- | --- |
| Shared streams + dual control planes boot | Pass | all three health checks return `ok` | any process never reaches health |
| `list_peers` in shared-stream topology | Pass | both `agent-a` and `agent-b` appear | only one peer appears |
| `prompt_peer` A -> B | Pass | response includes `hello across shared stream` | `peer '<name>' not found` or timeout |
| A/B state stream reflection | Pass | A has `child_session_edge`; B has echoed chunk | B never materializes a prompt turn or chunk |
| Exact isolated `fireline run` topology | Fail today | n/a | peer discovery fails because deployment discovery is not shared |
| W3C `traceparent` propagation | Fail today | n/a | B-side initialize lacks `_meta.traceparent` |

## Rough Edges And Workarounds

- **Rough edge:** the happy path is not the same as the naïve `fireline run` shape from the original ask.
  **Workaround:** use the shared-stream control-plane topology captured here.
- **Rough edge:** the script captures state-stream evidence, not the raw B-side ACP tap.
  **Workaround:** if raw wire proof is required in rehearsal, reuse the websocket tap technique from FQA-5 and keep it clearly labeled as instrumentation.
- **Rough edge:** the state stream URLs are stable, but ACP URLs are provision-time ephemeral websocket endpoints.
  **Workaround:** rely on the script's printed `summary.json` path and the persisted state-stream evidence instead of hard-coding ACP URLs in narration.

## Recorded Replay

Recorded replay: [pending — captured during rehearsal]

Intended filename:
- `docs/demos/assets/peer-to-peer-demo-capture-rehearsal-01.mp4`

Intended location:
- `docs/demos/assets/`
