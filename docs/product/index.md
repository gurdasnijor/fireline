# Product Docs

> Related:
> - [`../architecture.md`](../architecture.md)
> - [`../ts/primitives.md`](../ts/primitives.md)
> - [`../execution/README.md`](../execution/README.md)

This folder keeps product framing separate from implementation slices and
technical reference docs.

The goal is to answer:

- what product Fireline could become
- what user problems this architecture can solve
- how Fireline should show up inside real workflows and products
- which adjacent systems Fireline should complement rather than copy

## North Star

Fireline should become a **durable agent fabric**:

- every meaningful run becomes a durable session
- execution can move across local and remote runtimes
- agent capabilities can travel with the run
- cross-cutting agent behavior can be extracted into reusable conductor
  components
- users and operators can understand what happened from durable evidence alone

## Reading Order

- [`vision.md`](./vision.md)
  The high-level product thesis and why Fireline is well positioned.
- [`object-model.md`](./object-model.md)
  The main product objects: sessions, workspaces, capability profiles, runtimes,
  and runs.
- [`runs-and-sessions.md`](./runs-and-sessions.md)
  The boundary between the live run object and the durable session record.
- [`workspaces.md`](./workspaces.md)
  The portable working-context model for local folders, git sources, and
  snapshots.
- [`product-api-surfaces.md`](./product-api-surfaces.md)
  The higher-level API surfaces that should sit above the existing systems
  primitives.
- [`capability-profiles.md`](./capability-profiles.md)
  What a reusable capability profile should contain and how it should map to
  MCPs, credentials, skills, and policy defaults.
- [`out-of-band-approvals.md`](./out-of-band-approvals.md)
  How long-running runs should pause durably on gated actions and resume after
  later service.
- [`user-surfaces.md`](./user-surfaces.md)
  How end users and host products should actually interact with the system.
- [`ecosystem-story.md`](./ecosystem-story.md)
  How Fireline maps to ACP proxies, managed agents, MCP, `agent.pw`, and
  weaker harnesses such as OpenClaw-style systems.
- [`roadmap-alignment.md`](./roadmap-alignment.md)
  How the execution slices map to the product vision and how future slices
  should be chosen.
- [`backlog.md`](./backlog.md)
  Candidate spikes and slices that turn the product vision into a delivery
  backlog.
- [`priorities.md`](./priorities.md)
  What Fireline already has, what is missing, and where product energy should
  go next.

## Relationship To Technical Docs

These product docs sit above:

- [`../architecture.md`](../architecture.md)
- [`../ts/primitives.md`](../ts/primitives.md)
- [`../runtime/control-and-data-plane.md`](../runtime/control-and-data-plane.md)
- [`../execution/13-distributed-runtime-fabric/README.md`](../execution/13-distributed-runtime-fabric/README.md)

Those documents define how the system is built.

This folder defines what product value that architecture should deliver.
