# pi-acp to OpenClaw

> Status: roadmap demo, not a working tutorial
>
> This document describes the strongest Fireline story on the table: take a simple local ACP agent, wrap it in Fireline middleware, prove it locally, then turn the same file into an always-on cloud agent that can discover peers, share state, and participate in a larger operational surface.

## The pitch in one sentence

**The same 15-line agent definition should be able to move from "local experiment on my laptop" to "always-on cloud agent in a peer fleet" with nothing more than a flag change.**

That is the north star because it collapses three normally separate worlds into one:

- local agent experimentation
- production deployment
- multi-agent operations

Today, teams usually rebuild the agent between each phase. Fireline's job is to make that rewrite unnecessary.

## 1. What pi-acp is

[`pi-acp`](https://github.com/svkozak/pi-acp) is a simple ACP-compatible coding agent adapter. That makes it a good starting point for a Fireline demo because it is small, legible, and already speaks the protocol Fireline wants on the agent side.

It is useful in this story for exactly that reason:

- it is not a whole platform
- it is not a cloud product
- it is not already carrying orchestration baggage

That means the audience can see what Fireline adds around the agent instead of confusing Fireline with the agent itself.

## 2. What OpenClaw is

[OpenClaw](https://github.com/openclaw/openclaw) is a good reference point for the **shape** of the destination experience: always-on agents, gateway-owned sessions, queueing, streaming visibility, and operational surfaces that outlive any single client connection.

In other words, OpenClaw is not the inspiration for Fireline's internals. It is the inspiration for the **product feeling** we want the Fireline demo to achieve:

- agents stay up
- messages queue when work is already active
- sessions belong to the system, not to one browser tab
- operators can observe many agents from one place

The Fireline version should deliver that feel with a different substrate:

- ACP for the agent connection
- durable streams for state and replay
- Fireline middleware for control
- stream-backed peer discovery for cross-host visibility

## 3. The Fireline version

This is the north-star `agent.ts` file:

```ts
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve, budget, secretsProxy, peer } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
      GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' },
    }),
    peer({ peers: ['reviewer'] }),
  ]),
  agent(['pi-acp']),
)
```

This is the whole dream: one file that defines

- where the agent runs
- what guardrails apply
- how it is observed
- which secrets it can use
- which peers it can discover

without forcing the user to learn separate local-dev, cloud-deploy, and fleet-orchestration products.

## 4. The demo walkthrough

### Step 1: Local development

The operator starts with a local pi-acp agent and wraps it in Fireline:

```bash
npx fireline agent.ts
```

The audience sees:

- a local ACP endpoint come up
- the agent answer prompts
- trace events landing in Fireline's observation surface
- approval and budget middleware changing the agent's behavior without touching the agent itself

This is the "hello world" moment, but it is not a toy. The point is that the middleware is already live:

- `trace()` means the run is observable
- `approve()` means risky actions can pause
- `budget()` means the run is governed

The code did not change. Only the wrapper changed.

### Step 2: Test the middleware locally

The operator asks the agent to do something risky, like changing a sensitive file or proposing a destructive action.

The audience sees:

- the request pause instead of executing immediately
- a pending approval show up in the state surface
- the run resume when approval is granted

This matters because the approval is not a modal dialog trick. In Fireline, the approval is durable state. The process can restart and the pending approval still exists.

### Step 3: Deploy it to the cloud

Once the local behavior feels right, the operator changes only the launch mode:

```bash
npx fireline deploy agent.ts --provider anthropic --always-on
```

The audience should understand exactly what changed:

- not the agent
- not the middleware
- not the observation model
- only the deployment target

Now the same agent is no longer a local experiment. It is an always-on cloud worker.

This is the handoff story that matters: Fireline should make "local to remote" feel like promotion, not migration.

### Step 4: Add a peer

Now add a second agent:

```bash
npx fireline deploy reviewer.ts --provider anthropic --peer agent
```

The audience now sees a small agent fleet instead of one isolated worker.

The north-star behavior is:

- the reviewer can discover the main agent through Fireline's discovery stream
- the two agents can exchange work or annotations through peer routing
- both agents appear in one observation surface
- their interactions form one lineage graph instead of disconnected logs

### Step 5: Show the OpenClaw-style product surface

This is the punchline. The operator opens a control UI that shows:

- a shared message board for the agent fleet
- a lineage graph of cross-agent work
- approvals waiting on humans
- one live activity surface for the whole system

The important point is that Fireline does not need to ship a full OpenClaw clone to make this compelling. It only needs to prove that the substrate underneath that style of product is already there.

## 5. What Fireline primitives are doing the work

| Demo step | Fireline feature doing the work | Why it matters |
|---|---|---|
| Local pi-acp run | ACP harness + sandbox definition | Fireline can host a real ACP-speaking agent without changing the agent |
| Trace and approvals | Middleware + durable event stream | Control and observability are infrastructure, not prompt hacks |
| Local to cloud handoff | Shared durable-streams URL + remote provider story | The session state survives the move |
| Always-on cloud runtime | Provider abstraction | Local, container, and hosted execution share one model |
| Peer fleet | `hosts:tenant-<id>` discovery stream + `StreamDeploymentPeerRegistry` | Agents can find each other without a side registry |
| Shared message board / lineage graph | Stream-backed observation + projected UI state | The fleet can be observed as one system |

## 6. Why this demo is stronger than a normal agent demo

Most agent demos prove one of these things:

- the model is smart
- the UI looks polished
- the tool call works

This demo proves something harder:

**the same agent definition can survive the transition from prototype to system.**

That is strategically important because it answers the question every serious team asks after the first good prototype:

> "How much of this do we now have to rebuild for production?"

Fireline's best answer is:

> "Much less than you think."

## 7. What exists today vs what is still planned

This section matters. Without it, the demo reads like vapor.

### Exists today

- Fireline already has stream-backed cross-host discovery via the tenant discovery stream and `StreamDeploymentPeerRegistry`. Hosts and runtimes can discover each other across machines and providers as long as they are publishing into the same tenant discovery stream.
- Fireline already has durable approval plumbing in Rust. Approval requests and resolutions are written to the state stream and can survive process restart.
- Fireline already has a provider model in Rust, including local, Docker, microsandbox, and an Anthropic-backed provider implementation behind feature flags.
- Fireline already has client middleware builders for `trace()`, `approve()`, `budget()`, and `peer()`.

### Partially exists

- The approval story is real, but today's Rust approval component still gates at the **prompt** layer, not at the typed tool-call layer. The north-star demo wants "approve before destructive tool call," and that will be stronger once ACP/MCP tool interception is exposed cleanly.
- Peer discovery exists in the backend, but the north-star `peer({ peers: [...] })` story still needs the remaining client wiring and polished deploy UX described in `docs/gaps-declarative-agent-api.md`.
- The Anthropic provider exists in Rust, but the user-facing "one-file deploy to always-on cloud agent" flow is not yet a polished product surface.

### Does not exist yet

- `npx fireline agent.ts`
- `npx fireline deploy agent.ts --provider anthropic --always-on`
- `secretsProxy()` in the TypeScript client
- a packaged OpenClaw-style control UI
- a ready-made "shared message board" product surface

Those are the gaps between the current repo and the north star. They are product and SDK gaps, not a missing-systems-foundation gap.

## 8. Why the secrets story matters in this demo

The local-to-cloud handoff is not believable without credentials.

If the demo says "the same file runs locally and remotely," but the remote version cannot safely obtain its API keys, the story falls apart. That is why `secretsProxy()` matters so much in the north-star file.

The honest current state is:

- the Rust secrets component design is real and substantial
- the TypeScript middleware surface is not there yet
- the CLI-driven handoff path is not there yet

So the right way to frame this demo is:

**the secrets story is part of the north star, and the Rust side has enough shape to make it credible, but the user-facing API is still roadmap.**

## 9. Why the message board and lineage graph belong here

These are not random UI flourishes.

They prove the central Fireline claim: the same durable substrate that keeps the agent alive also gives you a shared operational view.

In this demo:

- the message board is a projected view over durable agent activity
- the lineage graph is a projected view over cross-agent relationships and work handoffs
- the operator does not need a separate telemetry system to understand what the fleet is doing

That is the difference between "agent runtime" and "agent infrastructure."

## 10. The actual punchline

The reason this should make someone say "I need to build on this" is simple:

today, most teams can get a local agent demo or a cloud agent product or a multi-agent system, but not all three without rewriting everything in the middle.

Fireline's north star is that you should be able to start with this:

```bash
npx fireline agent.ts
```

and grow into this:

```bash
npx fireline deploy agent.ts --provider anthropic --always-on
npx fireline deploy reviewer.ts --provider anthropic --peer agent
```

without changing the agent definition itself.

If Fireline can make that true, it becomes much more than a runtime. It becomes the bridge between experimentation and operations.

## Target architecture (post-canonical-identifiers)

This demo currently uses today's public surfaces: `compose(...).start()`, `FirelineAgent`, `fireline.db()`, and `appendApprovalResolved()`.

Those are the current runtime APIs, not the final architecture target.

For the identifier model Fireline is moving toward, see [ACP Canonical Identifiers](../proposals/acp-canonical-identifiers.md): agent-layer state should be keyed by canonical `SessionId`, `RequestId`, and `ToolCallId`, with no synthetic ids.

For the generalized workflow substrate behind the demo's approval handshake, see [Durable Subscriber](../proposals/durable-subscriber.md): approvals become one instance of a broader durable suspend / wake pattern.

For the eventual awakeable sugar that replaces today's ad-hoc resolve pattern, see [Durable Promises](../proposals/durable-promises.md).
