# Fireline Architecture

> Runtime substrate for hosting, tracing, and peering ACP-compatible agents.
> Pairs with Flamecast, the control plane that orchestrates those runtimes.

## What Fireline Is

Fireline runs an ACP conductor in front of an ACP-compatible terminal agent,
observes the protocol traffic that flows through that conductor, and persists a
durable trace that TypeScript consumers materialize into state.

It is deliberately narrow:

- It is not a control plane. Flamecast owns orchestration, scheduling,
  inventory, operator UX, and product policy.
- It is not a UI. Browsers, CLIs, and services consume Fireline through ACP and
  durable streams.
- It is not a query server. Rust does not own a materialized state API.
- It is not an agent execution OS. Fireline hosts agents and components, but it
  does not try to replace container, sandbox, or VM providers.

What Fireline does own:

- conductor composition
- transport adapters that expose the conductor over a wire or local attach
- durable trace production
- host-mediated peer calls
- helper endpoints that are tightly coupled to the host process

## Pairing with Flamecast

```text
Flamecast
  control plane
  - runtime orchestration
  - scheduling
  - permissions UX
  - operator-facing APIs

        consumes
           |
           v

Fireline
  runtime substrate
  - ACP conductor
  - transport adapters
  - durable trace
  - peer component
  - runtime hosting boundary

        hosts
           |
           v

ACP terminal agent
```

The boundary is intentional:

- Flamecast should be able to target any ACP-compatible runtime, but Fireline
  gives it a better substrate.
- Fireline should be usable without Flamecast, but it should not grow its own
  control-plane ambitions.

## Core Principles

### 1. Producer-only Rust posture

Rust produces durable protocol trace. It does not own the consumer read model.

That means:

- no Rust query API as a first-class product surface
- no Rust-owned entity schema as the source of truth
- no parallel Rust and TypeScript state systems trying to stay in sync

If a consumer wants state, it reads the trace stream and materializes it
locally in TypeScript.

### 2. TypeScript owns the consumer schema

Entity shapes such as prompt turns, chunks, permissions, and derived session
views live in TypeScript.

The expected shape is:

- `@fireline/state` defines the schema and projections
- schema artifacts can be exported for Rust conformance tests
- Rust emits protocol trace and validates against the published contract where
  needed

Rust should not invent a second canonical entity model.

### 3. Use the ACP SDK's composition model directly

Fireline should not build a custom proxy framework on top of the ACP SDK.

The core extension points are already there:

- `ConnectTo<R>` for active components and transport adapters
- `trace_to(WriteEvent)` for passive observation

Components such as the peer layer should implement the SDK's component model
and be composed into the conductor normally.

### 4. Use `trace_to(WriteEvent)` for observation

Observation is not a component concern.

When the system wants to record every protocol message that flowed through a
connection, it should use `trace_to(WriteEvent)` and append those observations
to the durable stream. That path is passive by design.

### 5. Use ACP `_meta` for protocol extensions

When Fireline needs to carry lineage or runtime extension data across ACP, it
stamps it into ACP `_meta`.

That applies to:

- peer-call lineage
- runtime provenance that needs to travel with ACP messages
- future Fireline-specific protocol extensions

Important nuance:

- ACP message extensions belong in `_meta`
- durable stream records may still wrap observed `TraceEvent`s with producer
  metadata such as `runtimeId` and `observedAtMs`

Those are different layers and should not be conflated.

### 6. Reusable transport-serving code belongs in `fireline-conductor`

There is no reusable product API server crate, but there is reusable
transport-serving code.

Examples:

- ACP over WebSocket
- local stdio attach via `sacp_tokio::Stdio`
- in-memory transport for tests
- MCP bridge HTTP/SSE listeners when they are part of protocol bridging

Those belong in `fireline-conductor` because they are protocol/transport
concerns, not product API concerns.

### 7. The binary owns process-level composition

The `fireline` binary owns:

- CLI parsing
- bootstrap sequencing
- signal handling
- router composition
- helper endpoints that are specific to the host process
- embedded durable-streams server instances
- connection lookup files

The binary mounts reusable transport handlers from `fireline-conductor`, but it
owns the overall process.

## Main Surfaces

Fireline exposes three architectural surfaces.

### ACP surface

The conductor presents as a single ACP-compatible agent regardless of the chain
behind it.

Clients should treat Fireline as the agent they connect to.

### Durable trace surface

Fireline appends trace records to durable streams. That trace is the durable
source of truth for observation and replay.

### Host helper surface

Small host-specific helper endpoints may exist where the host process must
mediate something that is not yet projected through ACP or the trace stream.
These are helper surfaces, not the architectural center.

## Crate and package shape

Rust:

- `fireline-conductor`
  - conductor assembly
  - transport adapters
  - durable trace sink / writer glue
- `fireline-peer`
  - peer component
  - peer discovery bootstrap descriptors
  - ACP-native peer invocation
- `fireline` binary
  - process composition
  - helper endpoints
  - embedded durable-streams server

TypeScript:

- `@fireline/client`
  - primitive ACP, stream, peer, host clients
- `@fireline/state`
  - schema, projections, local materialization, live queries

More detail lives in [`packages.md`](./packages.md).

## Runtime model

A Fireline runtime is the host process that owns:

- a conductor instance
- zero or more components
- one durable trace stream
- one or more transport adapters that expose the conductor

A control plane such as Flamecast should think in terms of runtime descriptors,
not raw transport URLs alone.

See [`runtime/provider-lifecycle.md`](./runtime/provider-lifecycle.md).

## Mesh model

Cross-agent work is mediated by the host through the peer component.

The agent sees tools such as:

- `list_peers`
- `prompt_peer`

Under the hood, Fireline should perform ACP-native peer calls and propagate
lineage through `_meta` so downstream observers can reconstruct the causal
graph from persisted streams alone.

See [`mesh/peering-and-lineage.md`](./mesh/peering-and-lineage.md).

## State model

Rust produces trace.

TypeScript consumes trace and materializes state.

That means the forward path is:

1. Fireline emits durable trace.
2. `@fireline/state` defines projections and local collections.
3. Consumers build dashboards, sinks, and UX on top of those local views.

See:

- [`state/consumer-surface.md`](./state/consumer-surface.md)
- [`state/session-load.md`](./state/session-load.md)
- [`ts/primitives.md`](./ts/primitives.md)

## Near-term execution path

The next delivery steps that fit this architecture are:

1. Extract and stabilize `fireline-conductor`.
2. Extract and stabilize `fireline-peer`.
3. Land the primitive TypeScript surface.
4. Implement ACP-native peer invocation with lineage.
5. Land runtime provider lifecycle.
6. Land consumer-side state materialization and `session/load`.

This order keeps Fireline small, honest, and aligned with Flamecast's needs.
