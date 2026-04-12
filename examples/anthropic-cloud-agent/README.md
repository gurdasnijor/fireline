# Anthropic Cloud Agent — Fireline as a Superset of Managed Agents

> Best-of-both-worlds demo: Anthropic's managed-agent infrastructure underneath, Fireline composition and durable observation on top.

## Pitch

Anthropic's managed agents give you the zero-ops cloud side: model hosting, remote execution, and managed infrastructure. Fireline adds the pieces Anthropic intentionally does not try to own:

- **Composition** via Fireline middleware around the agent
- **Observation** via `@fireline/state` over the durable stream
- **Portability** across providers without rewriting the harness shape

The result is a superset story:

- `provider: 'anthropic'` for managed cloud
- `provider: 'local'` for laptop development
- `provider: 'microsandbox'` for self-hosted production

The harness body stays the same. Only the provider hint changes.

## What this example demonstrates

1. Fireline composes a harness around an Anthropic-backed sandbox.
2. Fireline middleware still applies: `trace()`, `approve()`, and `budget()` are part of the harness spec, not part of Anthropic's provider.
3. `@fireline/state` still works because the handle exposes the same durable state endpoint regardless of provider.
4. ACP/session usage stays uniform because the handle exposes the same ACP endpoint regardless of provider.

## Why this is interesting

Anthropic's managed agents solve the infrastructure burden. Fireline solves the control-plane and composition burden:

- Anthropic gives you a managed cloud runtime.
- Fireline gives you a host that can add approval policy, tracing, and budget enforcement around that runtime.
- Fireline keeps the output observable in the same durable-state model your local and self-hosted providers already use.

That means you can prototype locally, deploy self-hosted when you need control, or flip to Anthropic's managed cloud when you want zero ops, without changing the surrounding app pattern.

## The code shape

The important line is the provider switch:

```ts
const handle = await compose(
  sandbox({ provider: 'anthropic' }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 500_000 }),
  ]),
  agent(['claude-sonnet-4-6']),
).start({
  serverUrl: 'http://localhost:4440',
  name: 'anthropic-cloud-agent',
})
```

The compiled example now uses the proposal surface directly: `compose(...)`,
`middleware([...])`, and `.start(...)`. The architectural point is the same:
the composed harness is provider-agnostic, and the provider changes only the
execution backend.

## State observation

Once the sandbox is provisioned, state observation is identical to every other Fireline provider:

```ts
const db = createFirelineDB({ stateStreamUrl: handle.state.url })
await db.preload()
```

Every session, prompt turn, approval request, and trace event becomes queryable through the same `@fireline/state` collections. Anthropic's managed cloud does not break Fireline's observation model because Fireline still owns the durable-state plane.

## ACP/session plane

The ACP/session side is also uniform:

- provision with Fireline
- connect to `handle.acp.url`
- create or load sessions with `@agentclientprotocol/sdk`
- observe state with `@fireline/state`

That keeps provider choice orthogonal to session logic.

## Running the demo

This example assumes:

1. a Fireline host is running at `http://localhost:4440`
2. that host has the Anthropic provider enabled server-side
3. the Anthropic provider bridges the remote runtime back into Fireline's ACP and durable-state endpoints

Type-check the example with:

```bash
pnpm exec tsc -p examples/anthropic-cloud-agent/tsconfig.json
```

## Why "superset" is the right word

Anthropic managed agents alone give you a managed execution substrate. Fireline wraps that substrate in a composable control plane:

- Anthropic infrastructure
- plus Fireline middleware
- plus Fireline durable observation
- plus Fireline provider portability

That is the "Claude's power + Fireline's composability" story this demo is meant to make concrete.
