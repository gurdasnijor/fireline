# Fireline Docs

Fireline is the runtime substrate that sits under Flamecast.

It hosts ACP conductors, exposes transport adapters, produces durable trace,
and mediates cross-agent calls. Flamecast remains the control plane above it.

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
- [`mesh/peering-and-lineage.md`](./mesh/peering-and-lineage.md)
  How Fireline nodes call each other over ACP while preserving durable lineage.
- [`state/consumer-surface.md`](./state/consumer-surface.md)
  How TypeScript consumers materialize state from Fireline trace.
- [`state/session-load.md`](./state/session-load.md)
  How reconnect and `session/load` fit into the model.

## Research

These are reference notes, not product contracts:

- [`research/adjacent-systems.md`](./research/adjacent-systems.md)
- [`research/agent-os.md`](./research/agent-os.md)
