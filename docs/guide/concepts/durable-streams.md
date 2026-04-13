# Durable Streams

Durable streams are the storage primitive underneath Fireline.

Every prompt request, chunk, approval, peer hop, and durable-subscriber completion eventually reduces to the same move:

- append an event to a stream
- save the next offset
- replay from that offset later

That is why Fireline treats the stream as the source of truth instead of treating host memory, sandbox state, or ad hoc HTTP endpoints as canonical.

## The Mental Model

Think in four verbs:

1. **append** a new fact
2. **replay** facts from the beginning or from a saved offset
3. **project** those facts into a read model
4. **resume** work by replaying the same stream after restart

Append-only matters here. Fireline does not "update the session row in place." It appends a later event that changes the projected view of the session.

That one choice gives Fireline three things at once:

- durable history
- restart-safe rebuilds
- multiple readers that can derive different views from the same log

## A Tiny Replay Example

Assume a local durable-streams service is running on `http://127.0.0.1:7474`.

```bash
STREAM='http://127.0.0.1:7474/v1/stream/guide-durable-streams'

curl -X PUT "$STREAM" \
  -H 'Content-Type: text/plain'

OFFSET=$(
  curl -si -X POST "$STREAM" \
    -H 'Content-Type: text/plain' \
    --data 'one' \
  | awk -F': ' '/^Stream-Next-Offset:/ {print $2}' \
  | tr -d '\r'
)

curl -X POST "$STREAM" \
  -H 'Content-Type: text/plain' \
  --data 'two'

curl -X POST "$STREAM" \
  -H 'Content-Type: text/plain' \
  --data 'three'

curl -X POST "$STREAM" \
  -H 'Content-Type: text/plain' \
  -H 'Stream-Closed: true'

curl -N "$STREAM?offset=$OFFSET&live=sse"
```

Expected output shape:

```text
event: data
data:two

event: data
data:three
```

What happened:

- the first append returned a `Stream-Next-Offset` cursor
- the later reader started from that saved cursor
- replay returned the exact suffix after `one`, not the full stream

Most Fireline users touch this through `fireline.db(...)`, approvals, or host-side subscriber drivers rather than raw `curl`. The point of the example is to show the primitive directly.

That suffix-replay contract is the thing Fireline builds on. `fireline.db(...)`, approval recovery, durable subscribers, and session resume all depend on the fact that a saved offset can later produce "everything after this point."

## What Fireline Actually Stores On Streams

For session work, Fireline writes typed envelopes onto per-session streams such as `state/session/{session_id}`.

Those envelopes include events like:

- `session_v2`
- `prompt_request`
- `chunk_v2`
- `permission_request`
- `approval_resolved`

Other Fireline subsystems use the same substrate for different domains:

- session and prompt history
- approval wait and resolution
- durable-subscriber delivery and completion
- peer discovery and other stream-backed indexes

Different projections read different subsets of the same append-only history. The storage primitive does not care whether the reader is a dashboard, a subscriber driver, or a restarted runtime.

## Why Replay Matters More Than "Persistence"

Persistence alone would only tell you that bytes still exist somewhere.

Replay gives you the stronger property Fireline needs:

- a fresh reader can reconstruct state without talking to the dead process that wrote it
- a restarted runtime can continue by reading prior events back in order
- a UI can catch up from its last saved cursor instead of polling bespoke endpoints

That is the real reason the stream is the source of truth. The stream is not just a log sink. It is the durable sequence any new reader can use to rebuild the same story.

## Offsets Are Cursors, Not Business IDs

Each read advances through the stream with an offset. Fireline saves and reuses those offsets as read cursors.

What an offset is for:

- resume a materializer or subscriber from the last observed point
- ask for the suffix after some known read edge
- prove that a reader has caught up to a specific append position

What an offset is not for:

- naming a session
- naming a prompt
- correlating business entities across systems

Business identity in Fireline comes from ACP-shaped ids such as `SessionId`, `RequestId`, and `ToolCallId`. Offsets are stream coordinates.

## Projections Are Derived Views

When you call `fireline.db(...)`, you are not querying the canonical state store directly. You are materializing collections from the stream:

- sessions
- prompt turns
- permissions
- chunks

That is why the observation plane is reactive and rebuildable.

It is also why a projection can be deleted and recreated without losing truth. The projection is a cached interpretation of the stream. The stream is the durable fact set.

## Append Order Beats Timestamps

In Fireline, append order is authoritative.

Timestamps are useful for UI and diagnostics, but the durable ordering contract comes from stream position. If two readers need to agree on what happened first, they follow append order, not local wall-clock time.

This is why tests and verification work talk about replay suffixes and offsets rather than asking readers to sort rows by `createdAt`.

## Gotchas

- Do not treat host memory as canonical state.
  If a fact matters after restart, it must be recoverable from the stream.
- Do not treat a projection as the durable source of truth.
  Collections and indexes are rebuildable views over the log.
- Do not confuse offsets with semantic ids.
  Offsets help readers resume; ACP ids identify the actual workflow entity.
- Do not assume retries are body-hash based.
  Durable-stream appends are producer-scoped and idempotent at the stream layer; if you retry incorrectly, dedupe semantics follow producer identity, not "same-looking payload."

## Read This Next

- [Observation](../observation.md)
- [Approvals](../approvals.md)
- [Durable Subscribers](../durable-subscriber.md)
- [Durable Promises](./durable-promises.md)
- [tests/managed_agent_session.rs](../../../tests/managed_agent_session.rs)
