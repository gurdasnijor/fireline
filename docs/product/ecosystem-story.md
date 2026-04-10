# Ecosystem Story

> Related:
> - [`vision.md`](./vision.md)
> - [`../programmable-topology-exploration.md`](../programmable-topology-exploration.md)
> - [`../mesh/peering-and-lineage.md`](../mesh/peering-and-lineage.md)
> - [ACP proxy chains](https://agentclientprotocol.com/rfds/proxy-chains)
> - [agent.pw](https://github.com/smithery-ai/agent.pw)

## How Fireline Maps To Ecosystem Solutions

Fireline should not try to clone every adjacent product directly.

It should map its primitives onto the jobs those products are solving.

### AGENTS.md, CLAUDE.md, and steering files

These are trying to provide:

- persistent instructions
- workspace-local context
- reusable guidance

Fireline's path:

- workspace-scoped context sources
- context-injection conductor components
- capability-profile defaults

### Hooks and response filters

These are trying to provide:

- request interception
- response shaping
- approval or policy checks

Fireline's path:

- ACP proxy/conductor components
- inbound transformers
- outbound transformers
- pause-and-escalate gates

### MCP servers

These are trying to provide:

- tool access
- external integrations
- structured actions

Fireline's path:

- MCP remains important
- conductor components can inject MCP servers and coordinate them with session
  context
- capability profiles can bind MCP definitions and secret references to the run

### Managed-agent platforms

These are trying to provide:

- durable sessions
- recoverable orchestration
- remote execution
- separation between orchestration and sandbox

Fireline's path:

- keep the session outside the harness
- keep the harness outside the runtime where possible
- use a control plane for lifecycle and inventory
- use runtimes as interchangeable execution hands
- keep durable evidence in the state stream

### Remote computer and sandbox products

These are trying to provide:

- a place to run code
- isolation
- access to files and tools

Fireline should integrate with these as runtime providers or runtime targets.

But the product should not collapse into "computer API for agents." The higher
value layer is the durable fabric above the computer:

- session continuity
- capability portability
- reusable extensions
- durable observation

## Conductor Components as Product Capabilities

One reason this broader product direction is plausible is that the conductor
architecture already maps well onto real user-facing capabilities.

Examples:

- `context_injection`
  - workspace instructions
  - carry-forward summaries
  - operator notes
  - profile defaults

- `audit`
  - compliance logs
  - playback inputs
  - exportable run history

- `peer_mcp`
  - specialist delegation
  - subagents
  - team-of-agents experiences

- approval, budget, and routing components
  - enterprise policy
  - safe automation
  - cost-aware orchestration

This is the bridge between low-level conductor design and high-level product
value.

## `agent.pw` Integration Story

[`agent.pw`](https://github.com/smithery-ai/agent.pw) is a credential vault
for agents that stores encrypted credentials, handles OAuth flows, and resolves
fresh auth headers at runtime from a stable connection path.

That lines up well with what Fireline should *not* try to own directly.

The strongest story is:

- Fireline owns sessions, runs, approvals, and runtime placement
- `agent.pw` owns credential storage, OAuth lifecycle, and just-in-time auth
  header resolution

### Product value of this pairing

- capability profiles can reference credential paths rather than raw secrets
- conductor-injected MCP bridges can resolve fresh auth headers at call time
- secrets stay outside the runtime when possible
- OAuth refresh and revocation stay in a purpose-built system

This is stronger than pushing long-lived raw credentials into every runtime.

## Out-of-Band Permission And Credential Flows

One especially strong product story is long-running agents that need approval
or authorization when the sponsoring human is no longer present.

Fireline can support this by:

- intercepting the gated action in a conductor component
- persisting a durable permission or authorization request
- exposing that request through a browser, mobile, Slack, or operator control
  plane
- pausing the run until the request is serviced
- resuming the run once approval or authorization is granted

That lets Fireline behave as if the run "blocks", while the actual wait is
durably externalized.

This matters for:

- long-running background agents
- OAuth or connect flows that need a human
- gated actions such as production deploys or financial operations
- agents that continue after the original interactive session is gone

## Strong Story For Weaker Harnesses

There is also a strong "upgrade layer" story for less-capable harnesses.

If a harness can speak ACP directly, or can be wrapped by a thin ACP adapter,
Fireline can add capabilities around it without requiring that harness to build
them natively.

That includes capabilities such as:

- durable sessions
- audit and replay
- context injection
- approval gates
- budget controls
- peer delegation
- lineage-aware multi-agent coordination

This is particularly relevant for simpler or more tool-limited harnesses,
including OpenClaw-style systems or internal harnesses that are useful but do
not yet have rich extension, persistence, or policy layers.

The story is not "replace the harness."

The story is:

- keep the harness where it is good
- use Fireline as the durable extension and orchestration layer around it

## Product Positioning Implication

This ecosystem story gives Fireline a sharper role:

- not another agent with its own bespoke tool format
- not just another sandbox launcher
- not just an MCP server

Instead:

**Fireline becomes the durable protocol and extension substrate that can make
existing harnesses, products, and workflows more capable without forcing each
of them to reinvent persistence, approvals, audit, policy, and orchestration.**
