# Session Load and Reconnect

## Purpose

`session/load` is the protocol-side reattachment primitive that lets a client
come back to an existing logical ACP session.

In Fireline, it should pair with durable state-stream replay.

## What `session/load` solves

It solves protocol reattachment:

- a client reconnects
- it loads an existing session by `sessionId`
- the runtime replays enough session history for the client to resume context

## What it does not solve alone

It does not, by itself, define:

- multiplayer/shared-session semantics
- who is controller vs observer
- how permission prompts are coordinated across multiple attached clients

Those are higher-layer concerns.

## Fireline model

The intended reconnect path is:

1. client reconnects to the runtime's ACP endpoint
2. client sends `session/load(sessionId)`
3. runtime or terminal agent replays session updates
4. client combines replayed ACP updates with durable state-stream replay as needed

## Capability negotiation

Not every underlying agent will support `loadSession` equally.

So Fireline should be explicit about:

- whether the downstream terminal agent advertises `loadSession`
- whether Fireline can offer a runtime-managed fallback
- what degraded mode means when neither exists

## Relationship to state-stream replay

State-stream replay and `session/load` solve different layers.

- `session/load` is the protocol-side reattachment primitive
- state-stream replay is the durable observation primitive

Fireline should use both, not choose one over the other.

## Relationship to mesh peering

Mesh peering can land before `session/load`, but long-running distributed work
will eventually want reconnect semantics.

That is why `session/load` is a follow-on foundation, not optional long-term.
