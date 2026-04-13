# Crash-Proof Agent

You kick off a release-readiness review. Ten minutes in, the first host is gone.

In most agent setups, that is the end of the story. The process died, the UI is
gone, and the user has to restate the work from scratch.

This example shows the Fireline recovery path instead:

- the first host starts the session
- the session state lives on a shared durable stream
- a replacement host starts against the same `stateStream`
- the replacement ACP connection calls `loadSession()`
- the same conversation continues under the same `sessionId`

That is the product claim this example is proving: the durable identity is the
session on the stream, not the first sandbox process.

## What The Example Does

The script runs a controlled handoff:

1. Start a session on the primary host.
2. Send the first review prompt.
3. Stop the first sandbox deliberately.
4. Start a replacement sandbox on the rescue host against the same
   `stateStream`.
5. Reattach with `loadSession({ sessionId, ... })`.
6. Send a follow-up prompt that tells the replacement host to continue without
   starting over.
7. Observe the durable result through `fireline.db(...)`.

Success means:

- one `sessionId`
- two different sandbox ids
- two completed prompt requests in one durable session history

## Why The Stop Is Controlled

This example uses `first.stop()` instead of a literal `kill -9`.

That is intentional.

The example is proving the session handoff contract, not rehearsing every
possible failure mode. A controlled stop keeps the public path deterministic:

- same harness
- same `stateStream`
- new host
- same `sessionId`
- explicit `loadSession()` call

For harsher restart-path status, see the deeper QA reviews and guides linked at
the end of this README.

## The Code

The core handoff is small:

```ts
const first = await harness.start({
  serverUrl: primaryUrl,
  name: 'crash-proof-primary',
  stateStream,
})
const acp1 = await first.connect('crash-proof-primary')
const { sessionId } = await acp1.newSession({ cwd: '/workspace', mcpServers: [] })

await acp1.prompt({
  sessionId,
  prompt: [{ type: 'text', text: firstPrompt }],
})

await acp1.close()
await first.stop()

const second = await harness.start({
  serverUrl: rescueUrl,
  name: 'crash-proof-rescue',
  stateStream,
})
const acp2 = await second.connect('crash-proof-rescue')

await acp2.loadSession({ sessionId, cwd: '/workspace', mcpServers: [] })
await acp2.prompt({
  sessionId,
  prompt: [{ type: 'text', text: secondPrompt }],
})
```

The non-negotiable parts are:

- both launches use the same `stateStream`
- the replacement connection keeps the original `sessionId`
- the replacement host calls `loadSession()` before the next prompt

Without those three pieces, you are starting a new conversation, not continuing
the old one.

## Observation Is Part Of The Demo

The example does not stop at `loadSession()`.

It also opens the durable state DB on the replacement host:

```ts
const db = await fireline.db({ stateStreamUrl: second.state.url })
```

Then it waits until `db.promptRequests.subscribe(...)` sees two completed prompt
requests for the same session before printing the final JSON summary.

That matters because Fireline's durability claim is not "trust the sandbox."
It is "the durable stream is the evidence."

## Run It

Prerequisites:

- Node `>=20`
- `pnpm`
- a shared durable-streams service
- two Fireline hosts pointed at that same durable-streams backend
- an ACP agent that supports `loadSession()`

From the repo root:

```bash
pnpm install
cargo build --bin fireline --bin fireline-streams --bin fireline-testy-load
```

In one terminal, start durable-streams:

```bash
./target/debug/fireline-streams
```

In a second terminal, start the primary host:

```bash
./target/debug/fireline --control-plane --port 4440 \
  --durable-streams-url http://127.0.0.1:7474/v1/stream
```

In a third terminal, start the rescue host:

```bash
./target/debug/fireline --control-plane --port 5440 \
  --durable-streams-url http://127.0.0.1:7474/v1/stream
```

In a fourth terminal, run the example:

```bash
cd examples/crash-proof-agent
pnpm install

FIRELINE_PRIMARY_URL=http://127.0.0.1:4440 \
FIRELINE_RESCUE_URL=http://127.0.0.1:5440 \
STATE_STREAM=crash-proof-demo \
AGENT_COMMAND=../../target/debug/fireline-testy-load \
pnpm start
```

## Why The Default Agent Is `fireline-testy-load`

This example needs an ACP agent that actually advertises and honors
`loadSession()` on current `main`.

`fireline-testy-load` is the deterministic choice in-repo today, so the example
defaults to it when `AGENT_COMMAND` is not set.

If you already have another ACP agent that supports `loadSession()`, point
`AGENT_COMMAND` at that instead.

This is the same reason the example does not default to a generic coding agent:
the handoff proof depends on resumability, not just on the ability to answer a
prompt.

## Expected Output

The script prints a JSON summary shaped like:

```json
{
  "question": "Can a replacement host continue the same agent session?",
  "primaryHost": "http://127.0.0.1:4440",
  "rescueHost": "http://127.0.0.1:5440",
  "stateStream": "crash-proof-demo",
  "sessionId": "session-...",
  "firstSandboxId": "runtime:...",
  "secondSandboxId": "runtime:...",
  "supportsLoadSession": true,
  "promptRequests": [
    {
      "requestId": 1,
      "state": "completed",
      "text": "Start a release-readiness review..."
    },
    {
      "requestId": 2,
      "state": "completed",
      "text": "You are now running on a replacement host..."
    }
  ]
}
```

What to check:

- `sessionId` stays the same
- `firstSandboxId` and `secondSandboxId` differ
- `supportsLoadSession` is true for the resumed session
- both prompt requests appear in one durable history

## Example Inputs

- `FIRELINE_PRIMARY_URL`
  Defaults to `http://127.0.0.1:4440`
- `FIRELINE_RESCUE_URL`
  Defaults to `http://127.0.0.1:5440`
- `STATE_STREAM`
  Defaults to `crash-proof-${Date.now()}`
- `WORKSPACE_DIR`
  Defaults to `/workspace`
- `FIRST_PROMPT`
  Optional override for the first task prompt
- `SECOND_PROMPT`
  Optional override for the follow-up prompt sent after handoff
- `AGENT_COMMAND`
  Defaults to `../../target/debug/fireline-testy-load`
- `OBSERVATION_TIMEOUT_MS`
  Defaults to `10000`

These are example-level inputs, not a replacement for Fireline's general env
surface.

## What This Example Proves Today

- a replacement host can provision the same harness against the same durable
  stream
- `loadSession()` can reattach the replacement host to the existing session
- the durable state stream still shows the original request history after the
  first sandbox is gone
- the user-visible conversation identity is the session, not the first runtime

## What It Does Not Prove

- that every ACP agent implements `loadSession()`
- that every literal host crash path is stage-green from the public path
- that Docker restart and reconnect semantics are uniformly green across every
  target

This example is the clean handoff proof. It is intentionally narrower than the
full restart-chaos story.

## Related Reading

- [Crash-Proof Agent guide](../../docs/guide/guides/crash-proof-agent.md)
- [Crash-Proof Sessions guide](../../docs/guide/guides/crash-proof-sessions.md)
- [Sessions and ACP](../../docs/guide/concepts/sessions-and-acp.md)
- [Observation model](../../docs/guide/concepts/observation-model.md)
- [Local to Cloud](../../docs/guide/guides/local-to-cloud.md)
