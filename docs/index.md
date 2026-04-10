# Fireline Docs

Fireline is the runtime substrate that sits under Flamecast.

It hosts ACP conductors, exposes transport adapters, produces durable
`STATE-PROTOCOL` streams, and mediates cross-agent calls. Flamecast remains the
control plane above it.

## Reading order

- [`architecture.md`](./architecture.md)
  The canonical statement of what Fireline is, what it owns, and what it does
  not own.
- [`packages.md`](./packages.md)
  The intended Rust crate and TypeScript package boundaries.
- [`ts/primitives.md`](./ts/primitives.md)
  The primitive-first TypeScript contract that projects Fireline's actual
  capabilities.
- [`runtime/provider-lifecycle.md`](./runtime/provider-lifecycle.md)
  How runtimes are created, addressed, and pinned to providers.
- [`runtime/lightweight-runtime-provider.md`](./runtime/lightweight-runtime-provider.md)
  How Fireline can borrow agentOS-style orchestration patterns for lightweight
  runtimes without adopting an in-process kernel.
- [`runtime/agent-catalog-and-launch.md`](./runtime/agent-catalog-and-launch.md)
  How Fireline discovers ACP agents, resolves launchable distributions, and
  launches chosen agents into runtimes.
- [`runtime/alchemy-docker-provisioning.md`](./runtime/alchemy-docker-provisioning.md)
  How a remote Docker-backed runtime provider could delegate substrate
  provisioning to Alchemy without moving runtime identity or discovery out of
  Fireline.
- [`execution/13-distributed-runtime-fabric/README.md`](./execution/13-distributed-runtime-fabric/README.md)
  The split execution plan for control-plane-backed runtime fabric work.
- [`mesh/peering-and-lineage.md`](./mesh/peering-and-lineage.md)
  How Fireline nodes call each other over ACP while preserving durable lineage.
- [`state/consumer-surface.md`](./state/consumer-surface.md)
  How TypeScript consumers materialize state from Fireline's durable stream.
- [`state/runtime-materializer.md`](./state/runtime-materializer.md)
  How Fireline maintains small runtime-local projections over the durable state
  stream without reviving a Rust-side consumer DB.
- [`state/session-load.md`](./state/session-load.md)
  How reconnect and `session/load` fit into the model.

## Product

- [`product/index.md`](./product/index.md)
  High-level product vision and reading order for product-facing docs.
- [`product/vision.md`](./product/vision.md)
  The durable-agent-fabric thesis and why Fireline is well positioned.
- [`product/object-model.md`](./product/object-model.md)
  Sessions, workspaces, capability profiles, runtimes, and runs.
- [`product/runs-and-sessions.md`](./product/runs-and-sessions.md)
  The difference between the managed run object and the durable session record.
- [`product/workspaces.md`](./product/workspaces.md)
  The portable working-context model for local paths, git sources, and
  snapshots.
- [`product/user-surfaces.md`](./product/user-surfaces.md)
  How end users and host products should actually interact with the system.
- [`product/ecosystem-story.md`](./product/ecosystem-story.md)
  How Fireline maps to ACP proxy chains, managed agents, `agent.pw`, and
  weaker harnesses.
- [`product/roadmap-alignment.md`](./product/roadmap-alignment.md)
  How the execution slices map to the product vision and how future slices
  should be chosen.
- [`product/backlog.md`](./product/backlog.md)
  Candidate spikes and slices that turn the product vision into a delivery
  backlog.
- [`product/priorities.md`](./product/priorities.md)
  What exists today, what is missing, and where product effort should go next.

## Research

These are reference notes, not product contracts:

- [`research/adjacent-systems.md`](./research/adjacent-systems.md)
- [`research/agent-os.md`](./research/agent-os.md)
