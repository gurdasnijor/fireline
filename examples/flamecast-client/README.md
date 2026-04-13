# Flamecast Client

`examples/flamecast-client` is the biggest user-facing example in this repo on
purpose. It is not a toy prompt runner. It is a reference dashboard for the
problem teams actually hit once they have more than one agent in flight:

**who is running, what are they doing, what needs approval, and how do I keep
the work moving without rebuilding an admin console from scratch?**

This example answers that question with a Fireline-backed control room:

- a home screen for starting sessions against reusable agent templates
- a sidebar for active sessions, previous sessions, runtimes, terminals, and
  queued follow-ups
- a session view with transcript state, approval cards, file previews, and
  runtime context
- a small server that translates Fireline handles into a product-shaped API for
  the dashboard

## Why This Example Exists

The small examples prove one primitive at a time. `flamecast-client` proves the
surface those primitives enable when you combine them:

- Fireline starts the runtime
- Fireline keeps the transcript durable
- Fireline exposes pending approvals as structured state
- Fireline keeps runtime metadata and session state visible enough to drive a
  real operator UI

The point is not that Flamecast is special. The point is that a product like
Flamecast should be able to stand up on Fireline without inventing a second
control plane.

## What Changed In This Refresh

- The default agent command now matches the landed ACP surface:
  `@agentclientprotocol/claude-agent-acp`
- The example no longer uses `secretsProxy`, which is intentionally out of
  scope for examples until `mono-4t4` lands
- The dashboard copy is framed as a product-facing reference console instead of
  an internal port
- The shell visuals are warmer and clearer so the example reads like a real
  operator surface

Credential note:

- if `ANTHROPIC_API_KEY` is present when you start the example server, this
  example passes it directly into the sandbox environment for the spawned ACP
  agent
- that is a deliberate temporary choice for example correctness
- it is not the long-term secret-handling story

## Run It

From the repo root:

```bash
pnpm install
cargo build --bin fireline --bin fireline-streams
target/debug/fireline-streams
target/debug/fireline --control-plane --port 4440 --durable-streams-url http://127.0.0.1:7474/v1/stream
pnpm --dir examples/flamecast-client dev
```

Open `http://127.0.0.1:3001`.

If you want the default Claude-backed template to work, export
`ANTHROPIC_API_KEY` before starting `pnpm --dir examples/flamecast-client dev`.

Useful overrides:

- `FIRELINE_URL`
  Fireline control-plane URL. Default: `http://127.0.0.1:4440`
- `PORT`
  Dashboard server port. Default: `3001`
- `FLAMECAST_WORKSPACE`
  Workspace path exposed to local runtimes. Default: current repo root
- `FLAMECAST_STATE_STREAM`
  Shared state-stream name for the example server's launched sandboxes
- `AGENT_COMMAND`
  Override the default ACP agent command

## What To Click First

1. Open the home screen and type a concrete task into the composer.
2. Pick a runtime, template, and directory if you want to override the
   defaults.
3. Send the first message. The dashboard creates a session and immediately
   queues the prompt into that session.
4. Open the runtime/session view to watch transcript state, file previews, and
   approval cards update in place.
5. Use the queue when a follow-up should wait until the session is idle or an
   approval is resolved.

## Current Boundaries

- This is a reference dashboard, not a hosted product.
- The server stores message queue state in memory.
- Runtime worktree creation is still stubbed with a TODO response.
- Secret handling is intentionally direct env injection for now, not
  `secretsProxy`.

Those limits are the right kind for an example: honest, small enough to read,
and still representative of the product shape.
