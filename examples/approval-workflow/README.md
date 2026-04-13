# Approval Workflow

You want the agent to move quickly, but you do not want the model to decide on
its own when a risky action is okay. A human or external system needs one
durable place to say "yes, continue" or "no, stop."

This example is the Fireline cookbook for that shape:

- `approve(...)` pauses the run on a durable approval request
- `fireline.db(...).permissions.subscribe(...)` notices the pending request
- a local webhook receiver stands in for the outside decision-maker
- `handle.resolvePermission(...)` records the decision and releases the same run

The important part is that the approval is durable state, not an in-memory
callback. If the reviewer UI, webhook worker, or agent process disappears, the
approval request still exists on the state stream.

## What This Example Shows

1. A launched agent is wrapped in `approve({ scope: 'tool_calls' })`.
2. `fireline.db({ stateStreamUrl }).permissions.subscribe(...)` sees the pending
   approval row as soon as Fireline writes it.
3. The example forwards that request to a local webhook receiver.
4. The receiver dedupes by `(sessionId, requestId)` and resolves the request
   through `handle.resolvePermission(...)`.
5. The original prompt resumes and the approval row stays visible on the
   durable state stream.

## The Code

```ts
const handle = await compose(
  sandbox({
    provider: 'local',
    fsBackend: 'streamFs',
    envVars: process.env.ANTHROPIC_API_KEY
      ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY }
      : undefined,
  }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
  ]),
  agent(agentCommand),
).start({ serverUrl, name: 'approval-workflow' })

db.permissions.subscribe((rows) => {
  const pending = rows.find((row) => row.state === 'pending')
  if (!pending) return

  void fetch(webhookUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      sessionId: pending.sessionId,
      requestId: pending.requestId,
    }),
  })
})
```

That is the product surface to remember:

- `approve(...)` creates the passive durable wait
- `fireline.db(...)` gives you the pending approval as durable state
- `resolvePermission(...)` writes the decision back on the same durable key
- the webhook hop is ordinary application glue, not special Fireline magic

## Run It

Prerequisites:

- a Fireline host reachable at `FIRELINE_URL` or `http://127.0.0.1:4440`
- Node `>=20`
- `pnpm`
- either:
  - `ANTHROPIC_API_KEY` for the default `claude-agent-acp` path, or
  - a local ACP test agent via `AGENT_COMMAND`

Install dependencies:

```bash
pnpm install
cd examples/approval-workflow
pnpm install --ignore-workspace --lockfile=false
```

Fast deterministic smoke on current `main`:

```bash
AGENT_COMMAND=/absolute/path/to/fireline/target/debug/fireline-testy-fs \
pnpm start
```

Use an absolute path here. The Fireline host resolves `AGENT_COMMAND`, not the
example directory.

This smoke path uses the local scripted ACP agent, so it proves the durable
approval handshake deterministically. The default Claude path keeps the
product-shaped prompt about writing a reviewed file into `/workspace`.

Default product-shaped run with Claude Agent ACP:

```bash
ANTHROPIC_API_KEY=... \
pnpm start
```

The script prints:

- the `sessionId`
- the Fireline `stateStream` URL
- the approval records seen by the webhook receiver
- the durable permission rows Fireline projected from the stream

## Why This Is The Right Example Shape

This example is intentionally not a custom polling loop. The app subscribes to
the durable state stream, sees a real pending approval row, and hands that row
to an outside decision-maker through a normal webhook.

That is the buyer-facing value:

- your agent can pause safely before risky work
- your approval UI can live in a webhook worker, dashboard, Slack bot, or
  ticket system
- the run resumes from the same durable workflow instead of starting over

## Honest Notes On Current `main`

- `approve({ scope: 'tool_calls' })` still lowers through the current
  prompt-level fallback path.
  This example proves the durable pause/resume workflow correctly, but the
  live match point is still broader than the public wording suggests.
- The webhook receiver is written to be idempotent.
  Durable approval delivery is still a retryable workflow, so the receiver
  dedupes by `(sessionId, requestId)` before calling `resolvePermission(...)`.
- This example avoids `secretsProxy()`.
  `mono-4t4` is still open, so the current safe path for the demo is direct
  `sandbox({ envVars })` or a local test agent override.
- `webhook(...)` is documented elsewhere, but this example stays on the
  currently proven path for `main`: approval gate plus state subscription plus
  explicit resolution.

## Read Next

- [Approvals](../../docs/guide/approvals.md)
- [Durable Subscribers](../../docs/guide/durable-subscriber.md)
- [Durable Promises](../../docs/guide/concepts/durable-promises.md)
