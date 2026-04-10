# Fireline as a Durable Agent Fabric

> Status: product / strategy note
> Type: higher-level framing doc
> Audience: maintainers deciding what product shape Fireline should grow into
> Related:
> - [`architecture.md`](./architecture.md)
> - [`ts/primitives.md`](./ts/primitives.md)
> - [`programmable-topology-exploration.md`](./programmable-topology-exploration.md)
> - [`runtime/control-and-data-plane.md`](./runtime/control-and-data-plane.md)
> - [`state/session-load.md`](./state/session-load.md)
> - [ACP RFD: Agent Extensions via ACP Proxies](https://agentclientprotocol.com/rfds/proxy-chains)
> - [Anthropic: Scaling Managed Agents: Decoupling the brain from the hands](https://www.anthropic.com/engineering/managed-agents)

## Purpose

This doc is not trying to define one more execution slice.

It answers a higher-level question:

**What product shape should Fireline grow toward if we want it to deliver the
outcomes people are asking for in adjacent conversations about managed agents,
remote workspaces, persistent sessions, MCP portability, and reusable agent
extensions?**

The key point is that the product opportunity is not "remote computer control"
by itself.

The opportunity is a **durable agent fabric**:

- sessions survive restart and remain inspectable
- execution can move across local and remote runtimes
- agent capabilities are portable across runtimes
- cross-cutting agent behavior is extracted into reusable conductor components
- observers and operators can understand what happened from durable logs alone

## Elevator Pitch

Fireline should become the substrate for **durable, portable, composable agent
runs**.

In practical terms:

- a run should persist as a session
- the same session should be able to work against local or remote execution
  environments
- the same agent capability bundle should travel with the run
- agent extensions should not be trapped inside one harness implementation
- execution environments should be interchangeable "hands", not the product's
  source of truth

That is the strongest interpretation of the ecosystem direction reflected in:

- ACP proxy chains: extract reusable agent extensions into protocol components
- managed agents: keep the session outside the harness and the harness outside
  the sandbox
- user feedback: "I want my folders, MCPs, secrets, and skills to come along"

## The Product Job To Be Done

The recurring user ask is not:

- "please expose a VM API"

It is closer to:

- "give me a durable agent environment I can reopen later"
- "let the agent run locally or remotely without me rebuilding everything"
- "preserve my context, tools, and policies across runs"
- "let me understand and trust what happened"
- "make extensions reusable instead of agent-specific"

That points to a product centered on **sessions, capabilities, and runtime
placement**, not on a specific sandbox technology.

## Why Fireline Is Well Positioned

Fireline already has a meaningful architectural advantage over a thin
"start-a-container and attach a WebSocket" approach.

It already owns:

- an ACP conductor in front of the terminal agent
- chainable ACP components via the conductor composition model
- passive protocol observation via trace writers
- durable `STATE-PROTOCOL` production
- host-mediated peer calls with lineage
- runtime-side materialization for coordination (`session/load`, active-turn
  lookup, future child-session lookup)

Those are unusually strong primitives for building a durable agent fabric.

They mean Fireline can do more than host an agent:

- it can **compose** behavior around the agent
- it can **observe** behavior durably
- it can **coordinate** behavior across runtimes
- it can **recover** from restart with durable evidence

That is the raw material needed for the higher-level product.

## The Conductor Advantage

The ACP proxy-chains RFD describes proxies as a universal extension mechanism:
components between client and agent that can intercept, transform, inject, and
coordinate ACP messages.

That aligns closely with Fireline's existing conductor architecture.

At a product level, this means Fireline can support reusable agent
capabilities that are currently fragmented across:

- `AGENTS.md` / `CLAUDE.md`-style instruction files
- editor rules and steering files
- hooks
- MCP servers
- bespoke harness logic
- subagent orchestration layers

The strategic claim is not that all of those disappear.

The strategic claim is that Fireline can provide a **common execution and
observation layer** underneath them:

- inbound prompt/context transformation
- outbound response transformation
- MCP tool injection
- approval and budget gates
- multi-agent routing
- audit and recording
- durable replay and lineage

That turns "extensions" from per-agent hacks into reusable components of the
fabric.

## Core Product Objects

If Fireline is going to feel like a durable agent fabric, it needs a stable
product vocabulary.

### 1. Session

The durable record of a run.

A session should answer:

- what happened?
- where did it run?
- can it be resumed?
- what child sessions or peer calls did it create?
- what artifacts, prompts, and outputs belong to it?

Important nuance:

Fireline already has the beginnings of this today:

- durable session rows
- runtime-side `SessionIndex`
- consumer-side `sessions` collection
- local `session/load` coordination

What is still missing is making `Session` a clear top-level product surface,
not just a row in the durable stream.

### 2. Workspace

The files and working context an agent operates against.

This is not the same thing as a runtime.

A workspace may be:

- a local folder
- a repo clone
- a synced snapshot
- a mounted project root

The point is to make "work against my code/data" portable across runtime
placements.

### 3. Capability Profile

The portable bundle of agent-facing capabilities and policy.

This is the answer to requests like:

- "it should have my configured MCPs"
- "it should have my secrets"
- "it should have my skills"
- "it should use this model/policy/tool budget"

A capability profile is distinct from both workspace and runtime:

- runtime says **where execution happens**
- workspace says **what files/context the run sees**
- capability profile says **what the run is allowed and able to do**

### 4. Runtime

The execution substrate.

Examples:

- local process
- Docker container
- Cloudflare container
- VM / microVM
- later, other provider-backed environments

The runtime is the "hands", not the durable source of truth.

### 5. Agent Run

The binding of:

- session
- workspace
- capability profile
- runtime placement

This is the object most users actually think they are creating when they "run
an agent."

## How Fireline Maps To Ecosystem Solutions

Fireline should not try to clone every adjacent product directly.

It should instead map its primitives onto the jobs those products are solving.

### AGENTS.md / CLAUDE.md / steering files

These are trying to provide:

- persistent instructions
- workspace-local context
- reusable guidance

Fireline's path:

- workspace-scoped context sources
- context-injection topology components
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
- capability profiles should bind MCP definitions and secret references to the
  run

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
- use Fireline runtimes as interchangeable execution hands
- keep durable evidence in the state stream

### Remote computer / sandbox products

These are trying to provide:

- a place to run code
- isolation
- access to files and tools

Fireline should integrate with these as runtime providers.

But the product should not collapse into "computer API for agents." The higher
value layer is the durable fabric above the computer:

- session continuity
- capability portability
- reusable extensions
- durable observation

## What Fireline Already Has

Fireline is not starting from zero on this product direction.

Already present today:

- durable `session`, `runtime_instance`, `prompt_turn`, and related state rows
- local `session/load` coordination
- runtime-owned session substrate
- lineage-aware peer calls
- programmable runtime topology for optional ACP components
- audit and context-injection as early proof components
- TypeScript-side state materialization over durable streams

This matters because it means the core story is already visible:

- a run can already be durably observed
- runtime behavior can already be modified by reusable conductor components
- the durable stream is already more important than the host-local side files

## What Is Still Missing

To deliver on the higher-level product ask, Fireline still needs four major
gaps closed.

### 1. Control-plane-backed runtime fabric

Today runtime lifecycle is still effectively local-first.

Fireline needs:

- a distinct control plane
- runtime registration and heartbeat
- provider-backed runtime lifecycle
- shared durable-streams deployment for many runtimes
- runtime-centric discovery

This is the basis for "my agent can run here or there."

### 2. First-class session product surface

Durable session state exists, but the product surface is still under-exposed.

Fireline needs a clearer answer for:

- list my sessions
- reopen this run
- inspect transcript/history/artifacts
- understand resumability
- track child sessions and handoffs

### 3. Workspace model

Fireline needs a real answer for:

- local folder now
- remote execution later
- same logical workspace across multiple runs

Without a workspace model, "move my folder somewhere and let the agent run on
it" remains ad hoc.

### 4. Capability-profile model

Fireline needs a portable answer for:

- MCP configuration
- secret references
- skills
- tool policy
- model defaults

Without this, users must rebuild their agent environment per runtime.

## Recommended Strategic Direction

The right product arc is:

1. Treat `Session` as the durable center.
2. Treat `Runtime` as interchangeable execution hands.
3. Treat `Workspace` as the portable file/context object.
4. Treat `CapabilityProfile` as the portable tool/policy object.
5. Treat conductor components as the reusable extension ecosystem.

That gives Fireline a clearer story than "agent hosting" alone:

- durable runs
- portable capabilities
- composable extensions
- provider-neutral execution
- observable and recoverable agent work

## What This Means For Near-Term Priorities

If the product goal is the conversation captured above, the next priorities
should be:

1. Finish the programmable-topology path and keep extending conductor-based
   components.
2. Land the distributed runtime fabric foundation so runtimes are no longer
   local-only.
3. Promote sessions from durable rows to a clearer product/API surface.
4. Define `Workspace` and `CapabilityProfile` as real objects before adding too
   many more runtime backends.
5. Keep secrets and external credentials outside the runtime wherever possible,
   with Fireline or adjacent control-plane services brokering access.

That ordering matters.

If Fireline only adds more runtime providers, it risks becoming "more ways to
launch sandboxes."

If it adds sessions, workspaces, capability profiles, and reusable conductor
extensions on top of a provider-neutral runtime fabric, it becomes a durable
agent substrate that can actually answer the user asks reflected in the
ecosystem conversations.

## Non-Goals

This document does not propose that Fireline should:

- become a full VM/kernel abstraction like agentOS core
- replace MCP
- own the entire control-plane product surface by itself
- turn the runtime into the durable source of truth
- make remote computer control the primary product abstraction

The goal is narrower and stronger:

**Fireline should be the durable substrate that lets agent sessions, portable
capabilities, and reusable extensions survive changes in harnesses, runtimes,
and deployment environments.**
