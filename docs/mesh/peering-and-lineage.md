# ACP Mesh Peering and Lineage

## Purpose

Fireline should let one runtime invoke another over ACP while preserving enough
lineage for downstream observers to reconstruct the causal graph from persisted
trace alone.

## Core question

Can one Fireline node invoke another over ACP, while preserving lineage that is
recoverable from persisted trace streams alone?

## Keep the tool surface, replace the wire

The agent-facing abstraction should stay host-mediated.

The agent sees tools such as:

- `list_peers`
- `prompt_peer`

The peer component owns:

- peer discovery
- ACP dial to the target node
- lineage propagation
- response streaming back into the local run

The wire underneath should be ACP-native, not helper REST.

## Per-call model

For the first implementation, a peer call should:

1. open a fresh ACP connection to the target node
2. send `initialize` with inherited lineage in `_meta`
3. create a new session on the child node
4. send `session/prompt`
5. stream updates to completion
6. close the child connection

This keeps the line between parent and child clear and makes lineage
connection-scoped.

## Identity model

There are two identity planes.

### Bootstrap plane

Used to find and address nodes:

- `nodeId`
- `acpUrl`
- `stateStreamUrl`
- optional helper API base

These may come from Flamecast, config, or a local registry.

### Durable plane

Used to answer causal questions after the fact:

- `runtimeId`
- `logicalConnectionId`
- `traceId`
- `parentPromptTurnId`

Design rule:

- bootstrap metadata may be out-of-band
- durable stitching must be reconstructible from persisted trace alone

## Lineage propagation

The parent runtime stamps lineage on the child connection's `initialize._meta`.

Expected fields:

- `fireline/trace-id`
- `fireline/parent-prompt-turn-id`
- `fireline/caller-node-id`
- `fireline/caller-runtime-id`

The child runtime uses that `_meta` as the inherited lineage context for the
entire child connection.

## Streams and observation

Start with per-node streams.

Observers such as Flamecast reconstruct the distributed graph by:

- reading one or more node streams
- joining on lineage fields

A single shared ingest may be useful later, but it should not be the required
first model.

## Deferred recovery

If a child node crashes mid-call:

- the lineage remains durable up to the crash
- automatic continuation is not solved by the mesh spike alone
- reconnect and continuation belong to the `session/load` story

That deferral should stay explicit.
