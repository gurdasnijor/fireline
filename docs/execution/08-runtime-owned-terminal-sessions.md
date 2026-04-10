# 08: Runtime-Owned Terminal Sessions

## Objective

Move terminal and session ownership above transient ACP transport attachments so
Fireline can support true cross-transport `session/load`.

## Why this is a separate slice

Slice 07 proves durable catalog lookup and honest non-resumable coordination.
It does not claim that a fresh terminal subprocess can resume a session.

To make that claim, Fireline must own:

- terminal lifetime
- session lifetime
- client attachment lifetime

independently.

## What this slice should prove

- a runtime can keep or recreate a terminal backend independently of one WebSocket
  client
- multiple ACP attachments can target the same underlying session backend over time
- `session/load` can succeed across disconnects without relying on agent-specific
  persistence semantics

## SDK investigation result

The ACP SDK already gives Fireline the transport primitive it needs.

What exists today:

- `agent_client_protocol_core::Lines::new(outgoing_sink, incoming_stream)`
- `agent_client_protocol_core::ByteStreams::new(outgoing, incoming)`
- `AcpAgent::connect_to(...)` already uses `Lines::new(...)` internally after
  spawning a subprocess

What does not exist today:

- transport-swap on an existing `ConductorImpl`
- a named `AcpAgent::from_streams(...)` helper

The absence of `AcpAgent::from_streams(...)` is not the real gap. Fireline can
already attach a terminal from existing stream pairs by building either:

- `Lines::new(...)` from a line sink/stream, or
- `ByteStreams::new(...)` from any `AsyncWrite` + `AsyncRead` pair

`ByteStreams` is the better terminal seam for slice 08. It is the lower-level,
SDK-supported transport primitive and avoids inventing a Fireline-specific
stream adapter concept.

The real limitation is conductor lifetime: `ConductorImpl::run(self, transport)`
consumes the conductor, so Fireline cannot keep one conductor alive and swap
new ACP transports into it later.

## Committed design direction

Slice 08 should use a Fireline-owned shared terminal and a fresh conductor per
ACP attachment.

Concretely:

- Fireline owns a runtime-scoped terminal handle
- the terminal subprocess outlives any single WebSocket ACP attachment
- each ACP connection still gets a fresh conductor
- the conductor instantiator closure attaches to the already-running terminal
  via SDK transport primitives, preferably `ByteStreams::new(...)`
- concurrent attachment is out of scope for this slice and should be rejected
  explicitly as `runtime_busy`

This keeps Fireline on the ACP SDK's session path while moving terminal
lifetime to the runtime layer.

## Why this over the alternatives

### Not transport-swap on an existing conductor

That would require an SDK change, and it is not actually the primitive Fireline
needs. The durable thing is the terminal backend, not the conductor instance.

### Not `AcpAgent::from_streams(...)`

That would only be a convenience wrapper around `Lines::new(...)`. It does not
unlock a new architecture.

### Not a Fireline-level ACP multiplexer

That is a much larger step aimed at concurrent attachments and multiplayer.
Slice 08 only needs single-attachment durability.

## Implementation shape

Fireline should introduce a small runtime-scoped shared-terminal primitive.

The recommended shape is a terminal actor, not raw stdio-handle loaning.

Why:

- it centralizes the single-attachment invariant
- it avoids drop/ownership hazards around `ChildStdin` / `ChildStdout`
- it gives one place to handle mid-turn disconnect cleanly
- it keeps terminal lifetime separate from conductor lifetime without inventing
  a new session engine

The actor should:

- bootstrap starts the terminal subprocess once
- runtime state owns the subprocess stdio permanently
- `/acp` asks the actor for an attachment
- if the terminal is already attached, the actor rejects with `runtime_busy`
- when the ACP connection drops, the actor clears the current attachment and
  keeps the subprocess alive

The actor loop should handle:

- subprocess stdout -> current attachment
- current attachment input -> subprocess stdin
- attach requests
- attachment drop/detach notifications
- runtime shutdown

The conductor remains per-connection and continues to use:

- `LoadCoordinatorComponent`
- `PeerComponent`
- `DurableStreamTracer`

The terminal becomes the only long-lived backend in the slice.

## Current checkpoint

Implemented in Fireline now:

- runtime-scoped shared terminal ownership
- fresh conductor per ACP attachment
- single-attachment enforcement with `runtime_busy`
- terminal reuse after ACP disconnect

Still not proven end-to-end in this repo:

- successful `session/load` reattach against a terminal that advertises and
  implements `loadSession`

That remaining proof depends on a suitable downstream agent capability, not on
more transport-plumbing work inside Fireline.

## Explicitly rejected designs

- raw stdio-handle loaning between conductors
- transport-swap on a live conductor
- a Fireline-level ACP multiplexer for concurrent clients
- a Fireline-owned session engine separate from the ACP SDK

## Upstream dependency

There is one small upstream SDK dependency for end-to-end testing:

- `agent_client_protocol_test::testy::Testy` should implement `loadSession`
- it should advertise `loadSession: true` during `initialize`

That is a test-agent capability PR, not a transport-plumbing PR.

Fireline should not grow a fake persistent test agent to work around that gap.

## Non-goals

- shared-session / multiplayer policy
- control-plane UX
- mesh-wide remote crash recovery
