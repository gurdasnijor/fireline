# Runtime Provider Lifecycle

## Purpose

Fireline needs a simple runtime lifecycle surface that hides provider-specific
bring-up details from Flamecast and other consumers.

The goal is to make these all look the same from the outside:

- local process
- Docker container
- E2B sandbox
- Daytona workspace

The runtime/provider surface is a bootstrap boundary, not the durable source of
truth. Provider adapters may keep small local records to find or relaunch
runtimes, but session and mesh durability still belong to the state stream.

## Core question

Can Fireline present a provider-agnostic runtime API that returns a stable
runtime descriptor with ACP and stream access, without leaking provider-specific
boot logic into the control plane?

## Runtime record

The durable unit is the runtime record, not the provider environment itself.

A runtime record should contain:

- `runtimeKey`
- `nodeId`
- `provider`
- `providerInstanceId`
- `status`
- `acpUrl` or local `acpTransport`
- `stateStreamUrl`
- `helperApiBaseUrl`
- `createdAt`
- `updatedAt`

If the backing environment dies, the record remains.

## Provider selection

Provider choice may be dynamic at creation time, but it is pinned afterwards.

Examples:

- `local`
- `docker`
- `e2b`
- `daytona`
- `auto` at creation time, resolved once to one of the above

Consumers should not have to re-run provider policy every time they reconnect.

## Lifecycle states

At minimum, Fireline should distinguish:

- `starting`
- `ready`
- `busy`
- `idle`
- `stale`
- `broken`
- `stopped`

This lets Flamecast reason about runtime health without conflating runtime state
with session durability.

## API surface

Primitive operations:

```ts
client.host.create(spec)
client.host.get(runtimeKey)
client.host.list()
client.host.stop(runtimeKey)
client.host.delete(runtimeKey)
```

The output is always a `RuntimeDescriptor`.

## Hosted vs local

The same runtime model should support both:

- hosted/networked runtimes that expose `acpUrl`
- local runtimes that hand back an attachable ACP transport

That is why bootstrap belongs in `client.host`, not `client.acp`.

Important boundary:

- local registries and discovery files are implementation details of a local
  provider adapter
- they must not leak into the TypeScript client contract
- they must not be used as durable session or mesh state

## Primary provider vs extension providers

Fireline should distinguish between:

- **primary runtime provider**
  - hosts the Fireline process itself
  - defines the runtime record
- **extension provider**
  - optional task-specific compute environment invoked from within the runtime
  - not itself the durable unit

This keeps the default runtime path simple and cheap while leaving room for
heavier sandbox integrations later.

## Bootstrap purity

The core runtime bootstrap should avoid guessing environment policy.

That means:

- identity inputs like `runtimeKey` and `nodeId` should be supplied by the
  caller or provider layer
- local default paths belong in the local provider adapter, not in the core
  bootstrap path
- provider-specific assumptions should stay at the provider edge

## Relationship to Flamecast

Flamecast should consume a stable runtime descriptor and not need to know:

- how Docker mapped ports
- how E2B or Daytona booted the environment
- whether the runtime was started locally or remotely

That is the point of this surface.
