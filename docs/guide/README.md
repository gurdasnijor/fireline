# Fireline Developer Guide

Fireline splits agent systems into three planes:

- control: define a serializable harness with `compose(...)`, then provision it
- session: talk to the running agent over ACP
- observation: treat the durable stream as the source of truth and materialize it into a live DB

These guides describe the code that exists in this repository today. Where the proposals are ahead of the implementation, the guides call that out explicitly.

## Guides

- [Concepts](./concepts.md)
  Core vocabulary: harness specs, handles, the three planes, and why durable streams are the source of truth.
- [CLI (`npx fireline`)](./cli.md)
  Run declarative agent specs with one command — boots streams, control plane, provisions the sandbox.
- [Compose and Start](./compose-and-start.md)
  How to define a harness, call `.start()`, and work with the `FirelineAgent` object it returns.
- [Middleware](./middleware.md)
  What each middleware helper actually emits today, how the Rust conductor interprets it, and what is still missing.
- [Observation](./observation.md)
  How to use `@fireline/state`, live collections, subscriptions, and the prebuilt query builders.
- [Approvals](./approvals.md)
  End-to-end approval flow: `approve(...)`, `permission_request`, durable waiting, and `approval_resolved`.
- [Resources](./resources.md)
  Resource refs, mount timing, and the current discovery story.
- [Providers](./providers.md)
  Local subprocess, Docker, Anthropic, and the current state of Microsandbox.
- [Multi-agent Topologies](./multi-agent.md)
  What `peer(...)`, `fanout(...)`, and `pipe(...)` actually do today.

## Primary source files

- [packages/client/src/sandbox.ts](../../packages/client/src/sandbox.ts)
- [packages/client/src/types.ts](../../packages/client/src/types.ts)
- [packages/client/src/middleware.ts](../../packages/client/src/middleware.ts)
- [packages/client/src/topology.ts](../../packages/client/src/topology.ts)
- [packages/state/src/collection.ts](../../packages/state/src/collection.ts)
- [packages/state/src/collections](../../packages/state/src/collections)
- [crates/fireline-harness/src/host_topology.rs](../../crates/fireline-harness/src/host_topology.rs)
- [crates/fireline-harness/src/approval.rs](../../crates/fireline-harness/src/approval.rs)
- [crates/fireline-sandbox/src/providers](../../crates/fireline-sandbox/src/providers)

## Upcoming primitives

These guides track the current runtime and package behavior. For the target
design that is stabilizing underneath them, see:

- [ACP Canonical Identifiers](../proposals/acp-canonical-identifiers.md)
  The governing identity contract for session/request/tool-call state and
  the agent-plane vs infrastructure-plane split.
- [Durable Subscriber Primitive](../proposals/durable-subscriber.md)
  The upcoming framework-level durable workflow primitive, including the
  webhook delivery profile in §5.2.
- [Durable Promises](../proposals/durable-promises.md)
  The planned imperative awakeable sugar layered on top of durable
  subscribers.
