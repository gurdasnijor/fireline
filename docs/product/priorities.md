# Product Priorities

> Related:
> - [`vision.md`](./vision.md)
> - [`object-model.md`](./object-model.md)
> - [`roadmap-alignment.md`](./roadmap-alignment.md)
> - [`../execution/13-distributed-runtime-fabric-foundation.md`](../execution/13-distributed-runtime-fabric-foundation.md)
> - [`../state/session-load.md`](../state/session-load.md)

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

This means the core story is already visible:

- a run can already be durably observed
- runtime behavior can already be modified by reusable conductor components
- the durable stream is already more important than host-local side files

## What Is Still Missing

To deliver on the product direction in this folder, Fireline still needs four
major gaps closed.

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
- inspect transcript, history, and artifacts
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

## Recommended Product Direction

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

## Near-Term Priorities

If the product goal is the direction captured in this folder, the next
priorities should be:

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
agent substrate that can answer the user asks reflected in adjacent ecosystem
conversations.

## Non-Goals

This product direction does not require Fireline to:

- become a full VM or kernel abstraction like agentOS core
- replace MCP
- own the entire control-plane product surface by itself
- turn the runtime into the durable source of truth
- make remote computer control the primary product abstraction

The goal is narrower and stronger:

**Fireline should be the durable substrate that lets agent sessions, portable
capabilities, and reusable extensions survive changes in harnesses, runtimes,
and deployment environments.**
