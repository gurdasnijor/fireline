# Fireline Packages

## Purpose

This document names the package and crate boundaries that fit the simplified
Fireline architecture.

The goal is not to create many packages. The goal is to isolate the real
concerns so runtime, peer, and consumer work can proceed without constant
collision.

## Rust crates

### `fireline-conductor`

Owns:

- conductor assembly
- reusable transport adapters
- state-writer wiring fed by ACP observation
- local attach helpers for stdio / in-memory testing

Does not own:

- peer policy
- TypeScript state schema
- product helper APIs

### `fireline-peer`

Owns:

- the peer component
- peer discovery descriptors
- MCP tool injection for peer calls
- ACP-native peer invocation
- lineage propagation across peer boundaries

Does not own:

- conductor transport hosting
- consumer-side state materialization
- control-plane inventory APIs

### `fireline` binary

Owns:

- process bootstrap
- CLI parsing
- signal handling
- router composition
- helper endpoints that are specific to the host process
- durable-streams embedding
- runtime-local state materializer wiring for narrow operational indexes

The binary should stay thin in concept even if it contains multiple modules.

## TypeScript packages

### `@fireline/client`

Primitive-first client package.

Owns:

- ACP client primitives
- stream subscription primitives
- peer-call client primitives
- runtime/bootstrap client primitives
- raw escape hatches

It should not try to be the polished ergonomic SDK yet.

### `@fireline/state`

Consumer-side state package.

Owns:

- schema definition
- state-stream ingestion
- local materialization
- live-query helpers
- sink helpers and adapters, if those remain close to state

It is the place where Fireline's consumer schema becomes concrete.

## Dependency direction

Rust:

```text
fireline-peer -----------.
                           \
                            > fireline binary
                           /
fireline-conductor -------'
```

TypeScript:

```text
@fireline/state   -> depends on the state-stream contract and consumer schema
@fireline/client  -> depends on transport/runtime/peer primitives
```

The main rule is conceptual rather than mechanical:

- Rust produces `STATE-PROTOCOL` and hosts transports.
- TypeScript consumes the state stream and defines the read model.

## Things that should not become packages yet

- a separate `fireline-state` Rust crate
- a Rust port of `@durable-streams/state` / `stream-db.ts` as a general
  consumer database package
- a separate `fireline-contracts` Rust crate, unless Rust truly needs a shared
  type seam that is still narrower than the current architecture
- a separate `fireline-server` crate
- a dedicated `webhooks` crate

These should remain modules until a real reuse boundary appears.

The allowed exception is a very small Rust-side projection helper if multiple
runtime-local indexes need to share replay/live-follow mechanics. That is an
internal operational helper, not a second canonical state layer.

## Why this boundary works

It matches the actual architectural seams:

- conductor substrate
- peer/data-plane behavior
- host process composition
- consumer-side TypeScript state

Anything more granular right now would add ceremony without buying clarity.
