# Product Vision

> Related:
> - [`../architecture.md`](../architecture.md)
> - [`../programmable-topology-exploration.md`](../programmable-topology-exploration.md)
> - [ACP proxy chains](https://agentclientprotocol.com/rfds/proxy-chains)
> - [Anthropic managed agents](https://www.anthropic.com/engineering/managed-agents)

## Elevator Pitch

Fireline should become the substrate for **durable, portable, composable agent
runs**.

In practical terms:

- a run should persist as a session
- the same session should be able to work against local or remote execution
  environments
- the same capability bundle should travel with the run
- cross-cutting agent behavior should be reusable across harnesses
- execution environments should be interchangeable "hands", not the product's
  source of truth

## The Product Job To Be Done

The strongest user ask is not:

- "give me a VM API"

It is closer to:

- "give me an agent environment I can reopen later"
- "let it run locally or remotely without rebuilding everything"
- "carry my context, tools, and policies with it"
- "let me understand and trust what happened"
- "make extensions reusable instead of product-specific hacks"

That points to a product centered on:

- durable sessions
- portable capabilities
- runtime placement
- reusable extension components

not on one specific sandbox technology.

## Why Fireline Is Well Positioned

Fireline already owns a strong set of primitives:

- an ACP conductor in front of the terminal agent
- chainable conductor components
- passive protocol observation via trace writers
- durable `STATE-PROTOCOL` production
- host-mediated peer calls with lineage
- runtime-local materialization for coordination and recovery

That means Fireline can do more than host an agent:

- it can compose behavior around the agent
- it can observe behavior durably
- it can coordinate work across runtimes
- it can recover behavior from durable evidence

Those are exactly the primitives a durable agent fabric needs.

## The Conductor Advantage

The ACP proxy-chains direction matters because it turns the conductor into a
universal extension layer.

That lets Fireline absorb concerns that are currently fragmented across:

- `AGENTS.md` / `CLAUDE.md`
- editor rules and steering files
- hooks
- MCP coordination
- custom harness logic
- multi-agent orchestration layers

At a product level, conductor components can become reusable capabilities for:

- context injection
- response transformation
- approval and budget gates
- MCP tool injection
- multi-agent routing
- audit and recording
- replay and lineage reconstruction

That is a stronger product story than "run an agent in a container."

## What This Means

The durable stream, the session, and the conductor chain are the real product
center.

Runtimes are important, but they should be treated as replaceable execution
hands under a more durable control, capability, and observation layer.
