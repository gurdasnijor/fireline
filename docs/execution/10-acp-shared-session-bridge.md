# 10: ACP Shared-Session Bridge

## Objective

Allow multiple ACP client transports to attach to the same runtime-owned session
backend without making the conductor itself long-lived or transport-swappable.

This is the Fireline analogue of the SDK's MCP bridge pattern:

- one long-lived backend
- one bridge/multiplexer in front of it
- many transport attachments managed by the bridge

## Why this is not slice 08

Slice 08 solves a narrower problem:

- the terminal outlives a transient ACP transport
- a new ACP attachment can reconnect to the same runtime-owned terminal
- only one attachment may own the terminal at a time

That is enough for durable `session/load` groundwork.

It does **not** define:

- concurrent ACP attachments
- observer vs controller roles
- how many clients may receive the same `session/update`
- who is allowed to send prompts, cancels, approvals, or MCP calls

Those are shared-session semantics, not transport-plumbing details.

## Why MCP bridge is a useful reference

The SDK's MCP bridge shows a valuable architectural pattern:

- the conductor still talks to one logical peer
- the bridge owns transport-side multiplexing
- the bridge can accept many external requests and route them onto a stable
  internal channel

In other words:

- the long-lived thing is the backend channel/actor
- the bridge, not the conductor, handles many external connections

That pattern is directly relevant to future ACP shared-session work.

## Why ACP is harder than MCP

ACP attachments are not just stateless request/response transports.

Each ACP client may:

- initialize capabilities
- create or load sessions
- receive streamed `session/update`
- send prompts
- send cancels
- resolve permission requests

So an ACP bridge cannot just "fan many transports into one channel" and call it
done. It also needs explicit policy.

## Proposed architecture

When Fireline is ready for shared attachments, add an `acp_bridge` layer in
front of runtime-owned session backends.

The shape should be:

- runtime owns the long-lived terminal and session backends
- an ACP bridge actor owns attachment registration
- each ACP client connection registers with the bridge
- the bridge routes inbound ACP requests to the correct runtime-owned session
  backend
- the bridge fans outbound `session/update` notifications to all registered
  attachments for that session

The conductor does not become long-lived and does not grow transport-swap.

Instead:

- conductors remain per-attachment or per-operation
- the bridge owns multiplexing and policy
- runtime-owned session state remains the durable center

## Recommended split inside the bridge

The bridge should separate:

### 1. Attachment registry

Tracks:

- `attachmentId`
- `clientId`
- transport handle
- attached `sessionId`
- role (`controller` or `observer`)
- last seen timestamp

### 2. Session routing

Maps:

- `sessionId -> runtime-owned backend`
- `sessionId -> attached clients`

### 3. Policy layer

Decides:

- who may create/load a session
- whether multiple controllers are allowed
- whether observers may send interactive methods
- how permission prompts are surfaced

### 4. Fanout layer

Broadcasts:

- `session/update`
- terminal output
- state-change notifications if needed

to the right attached clients.

## Relationship to current slice 08

Slice 08's `SharedTerminal` should remain valid under this future design.

It gives Fireline:

- runtime-owned terminal lifetime
- explicit attach/detach handling
- a place to reject unsupported concurrency today

Later, the ACP bridge can sit above that runtime-owned backend and replace the
current `runtime_busy` rejection with explicit shared-session policy.

## Relationship to durable state

The ACP bridge should not become a hidden source of truth.

Anything that matters after restart must still be projected into the durable
state stream, including eventually:

- attachment records
- controller/observer roles if they matter durably
- child-session edges across nodes

The bridge is a live coordination layer, not a durable data store.

## Entry criteria

Do not start this slice until all of the following are in place:

- slice 08 runtime-owned terminal/session lifetime is stable
- Fireline can do honest `session/load` against a resumable downstream terminal
- child-session topology across nodes is modeled durably

Without those, a shared ACP bridge will be built on unstable session identity.

## Acceptance criteria

This slice should eventually prove:

- two ACP clients can attach to the same logical session
- one may act as controller while another observes
- both receive coherent `session/update` fanout
- reconnect works without inventing a parallel Fireline session protocol
- durable state remains the source of truth for recovery and queryability

## Explicit non-goals

- rewriting the ACP SDK session engine
- transport-swap on a live conductor
- bypassing durable state with bridge-local truth
