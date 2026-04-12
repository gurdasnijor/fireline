# Fireline Developer Guide

Fireline splits agent systems into three planes:

- control: define a serializable harness with `compose(...)`, then provision it
- session: talk to the running agent over ACP
- observation: treat the durable stream as the source of truth and materialize it into a live DB

These guides describe the code that exists in this repository today. Where the proposals are ahead of the implementation, the guides call that out explicitly.

## Guides

- [Concepts](./concepts.md)
  Core vocabulary: harness specs, handles, the three planes, and why durable streams are the source of truth.
- [Compose and Start](./compose-and-start.md)
  How to define a harness, start it, connect to ACP, and manage lifecycle with `SandboxAdmin`.
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
