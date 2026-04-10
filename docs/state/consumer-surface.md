# Consumer State Surface

## Purpose

Fireline's canonical consumer contract is a durable stream of
`STATE-PROTOCOL` change messages.

TypeScript consumers should materialize local collections from that stream.

## Ownership

Rust owns:

- observation of ACP traffic via `trace_to(WriteEvent)`
- correlation needed to project ACP traffic into normalized entity changes
- durable append of `STATE-PROTOCOL` messages

TypeScript owns:

- schema definition
- stream-db ingestion
- materialized collections
- derived queries
- sink adapters

## Package shape

`@fireline/state` should own:

- the schema for Fireline's normalized entity collections
- `createFirelineDB(...)`
- derived live-query collections
- strict fixture-based conformance tests against real Rust-emitted NDJSON

It should not depend on a Rust state server.

## Input contract

The input is a durable stream of `STATE-PROTOCOL` change messages.

At minimum the initial Fireline producer should emit:

- `connection`
- `prompt_turn`
- `runtime_instance`
- `chunk`

The schema also reserves:

- `pending_request`
- `permission`
- `terminal`

Those can be added incrementally without changing the stream protocol.

## Output contract

Consumers should be able to build local collections such as:

- `connections`
- `promptTurns`
- `pendingRequests`
- `permissions`
- `terminals`
- `runtimeInstances`
- `chunks`

These collections are defined in TypeScript and synchronized by
`@durable-streams/state`.

## Query model

Queries stay in TypeScript.

The intended path is:

1. subscribe to the Fireline state stream
2. materialize local collections with `createFirelineDB(...)`
3. build live queries and sink adapters over those collections

## Webhooks and sinks

Webhooks are not a first-class architectural primitive.

They are one example of:

- subscribe to state
- filter or derive
- deliver to a sink

That same pattern should support:

- webhooks
- Slack notifications
- metrics
- audit exports

Fireline does not currently ship a dedicated Rust webhook forwarder module.
Webhook delivery remains a deferred sink composition on top of the state stream,
with TypeScript as the preferred consumer-side implementation surface.

## Relationship to ACP `_meta`

ACP `_meta` remains the protocol-extension channel for active components such as
peer calls and lineage propagation.

If those extensions need to become queryable state, the Rust producer projects
them into normalized `STATE-PROTOCOL` rows. Consumers should not parse raw ACP
messages as their primary contract.

## Relationship to helper APIs

Host helper APIs may exist for operational convenience, but they should not be
the canonical state contract.

The forward direction is:

`STATE-PROTOCOL stream -> local materialization -> app queries`

not:

`REST snapshot -> client polling`
