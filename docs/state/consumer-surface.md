# Consumer State Surface

## Purpose

Fireline produces durable trace. TypeScript consumers should materialize state
from that trace locally.

This document defines the intended consumer-side state surface.

## Ownership

Rust owns:

- durable trace production
- transport exposure
- component-level protocol behavior

TypeScript owns:

- schema definition
- trace ingestion
- materialized collections
- derived queries
- sink adapters

## Package shape

`@fireline/state` should own:

- the schema for Fireline's consumer entities
- trace record ingestion
- local materialization
- query helpers
- optional sink helpers

It should not depend on a Rust state server.

## Input contract

The input is Fireline trace.

At minimum each trace record should provide:

- the observed ACP event
- `runtimeId`
- `observedAtMs`
- any lineage metadata required for cross-node stitching

## Output contract

Consumers should be able to build local collections such as:

- connections
- prompt turns
- chunks
- permissions
- terminals

Those collections are consumer-owned views, not producer-owned wire types.

## Query model

Queries should stay in TypeScript.

The ideal shape is:

- ingest trace into local collections
- expose live-query helpers over those collections
- let applications build dashboards, operator tools, and sinks without polling

## Webhooks and sinks

Webhooks are not a first-class architectural primitive.

They are one example of:

- subscribe to state or trace
- filter/project
- deliver to a sink

That same pattern should support:

- webhooks
- Slack notifications
- metrics
- audit exports

## Relationship to Fireline's helper APIs

Host helper APIs may exist for operational convenience, but they should not be
the canonical state contract.

The forward direction is trace -> local materialization, not REST snapshot ->
client polling.
