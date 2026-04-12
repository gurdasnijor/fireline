# Durable Streams Protocol Alignment

## TL;DR

Fireline is mostly aligned with the upstream Durable Streams base protocol and
STATE-PROTOCOL, but the audit found four correctness gaps:

- `crates/fireline-sandbox/src/providers/anthropic.rs:405-416` writes
  `headers.operation = "append"` into the shared state stream. That is not a
  valid STATE-PROTOCOL change operation; upstream only allows `insert`,
  `update`, or `delete`.
- `crates/fireline-harness/src/trace.rs:74-88` and
  `crates/fireline-harness/src/audit.rs:172-175` append to a producer without
  `flush()` or `on_error(...)`. Those writes rely on background batching, so
  append failures are not surfaced at the call site.
- `crates/fireline-host/src/bootstrap.rs:332-347` and
  `crates/fireline-harness/src/host_topology.rs:372-387` retry `create_with()`
  on every error until a timeout, including permanent `409 Conflict` config
  mismatches that the protocol expects callers to treat as terminal.
- Fireline does not currently use the Rust client's `LiveMode::Auto` reader
  fallback or the producer-side recovery/backpressure options
  (`on_error`, `auto_claim`, `linger`, `max_in_flight`, `epoch`) documented by
  the upstream client.

Everything else falls into one of two buckets:

- correct STATE-PROTOCOL usage on the shared state stream
- correct base-protocol usage on non-state streams (deployment discovery,
  resource registry, audit, blob streams)

## Upstream References

- Base protocol:
  `https://github.com/durable-streams/durable-streams/blob/main/PROTOCOL.md`
- State envelope protocol:
  `https://github.com/durable-streams/durable-streams/blob/main/packages/state/STATE-PROTOCOL.md`
- Rust client:
  `https://github.com/durable-streams/durable-streams/tree/main/packages/client-rust`

The audit used the upstream requirements that constrain Fireline directly:

- Base protocol:
  - streams are created via `PUT`; if the stream already exists with matching
    configuration the server must return `200 OK`, otherwise `409 Conflict`
  - stream `Content-Type` is set at creation time
  - append requests with a body must use the stream's configured content type
- STATE-PROTOCOL:
  - state streams use `Content-Type: application/json`
  - change messages must contain `type`, `key`, `headers.operation`
  - `headers.operation` must be one of `insert`, `update`, `delete`
  - `value` is required for `insert` and `update`; optional for `delete`
- Rust client:
  - `LiveMode::Auto` prefers SSE and falls back to long-poll
  - producer error handling and batching/recovery features are exposed through
    `on_error`, `auto_claim`, `linger`, `max_in_flight`, and `epoch`

## Scope

Searches were run over `crates/`, `src/`, `packages/`, and `tests/`, then the
production call sites were reviewed manually:

- `rg -n "durable_streams::|DurableStream|Producer|Client.*durable|append_json|flush\\(\\)|stream_client|stream_handle|create_with|CreateOptions" crates src packages tests`
- `rg -n "content\\.type.*json|content_type|set_content_type" crates src packages tests`

This note focuses on production code in `crates/`, `src/`, and the shipped
TypeScript helper in `packages/client`. Tests are not enumerated below.

## Call-Site Findings

| File:line | Stream kind | Protocol alignment | Notes |
|---|---|---|---|
| `crates/fireline-host/src/bootstrap.rs:155-182,332-360` | Shared state stream + deployment discovery stream | Mixed | `application/json` is set on both stream handles and producers, and deployment events are flushed after append. `state_stream` hosts STATE-PROTOCOL messages via other helpers; `host_stream` carries plain `DeploymentDiscoveryEvent` JSON, which is fine because it is not a STATE-PROTOCOL stream. The gap is `ensure_stream_exists()`: it retries every `create_with(CreateOptions::new().content_type("application/json"))` error until timeout, so a terminal `409 Conflict` is masked as transient work. |
| `crates/fireline-harness/src/trace.rs:74-88,107-190` | Shared state stream | Mixed | The explicit helpers (`emit_host_instance_started`, `emit_host_instance_stopped`, `emit_host_spec_persisted`, `emit_host_endpoints_persisted`) use JSON streams, emit valid STATE-PROTOCOL envelopes on the wire (`#[serde(rename = "type")]`, `key`, `headers.operation`, `value`), and flush. The hot-path tracer (`new_with_host_context`, `write_event`) only calls `append_json(...)`; it never flushes and does not attach `on_error(...)`, so producer failures for ordinary projected events are not observed directly. |
| `crates/fireline-harness/src/state_projector.rs` | Shared state stream | Aligned | This is the canonical Fireline STATE-PROTOCOL projector. Its serialized envelopes use wire field `type`, include `key`, and encode the operation inside `headers.operation`. Legacy entity names such as `runtime_spec` are domain-specific but protocol-valid. |
| `crates/fireline-harness/src/audit.rs:172-175` | Dedicated audit stream | Mixed | Audit records are plain JSON event records, not STATE-PROTOCOL, which is fine for a dedicated audit stream. The correctness gap is durability/error visibility: the durable-stream sink only calls `producer.append_json(record)` with no `flush()` and no `on_error(...)`. |
| `crates/fireline-harness/src/approval.rs:181-186,266-285,304-340` | Shared state stream | Mostly aligned | `emit_permission_request()` writes a valid STATE-PROTOCOL envelope (`type = "permission"`, composite key, `headers.operation = "insert"`, `value = PermissionEvent`) and flushes, mapping flush failures into a real error. The replay reader and approval waiter both decode JSON array chunks correctly. Reader choice is conservative rather than optimal: `wait_for_approval()` uses `LiveMode::Sse`, not `LiveMode::Auto`, so it does not opt into the Rust client's SSE-to-long-poll fallback. |
| `crates/fireline-harness/src/host_topology.rs:148-153,358-387` | Named audit/topology streams | Mixed | Named streams are created with `CreateOptions::new().content_type("application/json")`, and named producers also set `application/json`, which matches the base protocol. Like bootstrap, `ensure_stream_exists()` retries every error until timeout instead of failing fast on `409 Conflict` mismatches. |
| `crates/fireline-orchestration/src/child_session_edge.rs:41-72` | Shared state stream | Aligned | Writes a valid STATE-PROTOCOL `child_session_edge` insert envelope and flushes after append. Errors from `flush()` propagate back to the caller. |
| `crates/fireline-tools/src/lib.rs:178-221` | Shared state stream | Aligned | `emit_tool_descriptor()` and `emit_tool_descriptors()` emit correct STATE-PROTOCOL envelopes (`type = "tool_descriptor"`, stable key, `insert`) and flush before returning. |
| `crates/fireline-resources/src/publisher.rs:97-134` | Dedicated resource event stream | Mostly aligned | The resource stream is intentionally a plain JSON event log of `ResourceEvent`, not STATE-PROTOCOL. The code creates the stream with `application/json`, builds producers with matching content type, appends, and flushes. `ensure_json_stream_exists()` is stricter than bootstrap because it treats `StreamError::Conflict` as terminal, but it still sleeps once before returning that error. |
| `crates/fireline-resources/src/index.rs:73-118` | Dedicated resource event stream | Aligned | Confirms the resource stream schema is a tagged JSON event log (`#[serde(tag = "type")]`), not a STATE-PROTOCOL change stream. That is valid because the stream is consumed by `ResourceRegistry`, not by `StateMaterializer`. |
| `crates/fireline-resources/src/registry.rs:206-330` | Dedicated resource event stream reader | Mostly aligned | Reads from `Offset::Beginning` with `LiveMode::Sse`, rebuilds the reader on retryable errors, and validates chunk/event JSON before projection. This is solid base-protocol usage. The only upstream feature gap is that it does not use `LiveMode::Auto`, so it does not leverage the client’s documented SSE-to-long-poll fallback. |
| `crates/fireline-resources/src/fs_backend.rs:84-102,243-263,298-388` | Shared state stream | Aligned | `fs_op` and `runtime_stream_file` are emitted as proper STATE-PROTOCOL updates on an `application/json` stream. `handle_write_text_file()` and `StreamFsFileBackend::write()` flush after append, so write failures are surfaced. The catch-up reader uses `LiveMode::Off`, which is appropriate for point-in-time replay. |
| `crates/fireline-resources/src/mounter.rs:159-180` | Blob stream reader | Aligned | Reads a durable blob stream as raw bytes from `Offset::Beginning` with `LiveMode::Off`. This is base-protocol usage, not STATE-PROTOCOL, and is correct for `DurableStreamBlob` resources. |
| `crates/fireline-sandbox/src/stream_trace.rs:20-75` | Shared state stream | Aligned | Duplicates the explicit host-spec/endpoints persistence helpers: matching JSON content type, valid STATE-PROTOCOL envelopes, flush after append. |
| `crates/fireline-sandbox/src/providers/anthropic.rs:375-418` | Shared state stream | Misaligned | The stream and producer use `application/json`, and the code flushes each append. The problem is the envelope shape: it writes `headers.operation = "append"` into the shared state stream. Upstream STATE-PROTOCOL only allows `insert`, `update`, or `delete`. Fireline’s own `StateMaterializer` enforces that and will skip these events as unsupported (`crates/fireline-session/src/state_materializer.rs:123-129,195-197`). |
| `crates/fireline-session/src/state_materializer.rs:1-17,121-129,195-197,243-277` | Shared state stream reader | Mostly aligned | The materializer explicitly implements STATE-PROTOCOL, accepts only `insert`/`update`/`delete`, and tolerates malformed neighbor events in a chunk. That matches the upstream state format. The live reader uses `LiveMode::Sse`, not `LiveMode::Auto`, so it does not use the Rust client’s documented fallback mode. Retry handling is partially manual: on retryable read errors it stays in the loop and asks the same reader for the next chunk again. |
| `crates/fireline-tools/src/peer/stream.rs:361-421` | Deployment discovery stream reader | Mostly aligned | The deployment discovery stream is intentionally a plain JSON event stream of `DeploymentDiscoveryEvent`, not STATE-PROTOCOL. The projection loop rebuilds the reader on retryable errors, which is correct. Like other long-lived readers, it uses `LiveMode::Sse` rather than `LiveMode::Auto`. |
| `src/main.rs:537-554` | Blob upload stream | Aligned | `upload_blob_stream()` uses `create_with()` plus `content_type(...)`, `initial_data(...)`, and `closed(true)`. That matches the base protocol’s create-with-initial-body and create-closed semantics for a one-shot blob stream. |
| `packages/client/src/events.ts:17-31` | Shared state stream | Aligned | The shipped TS helper appends a JSON STATE-PROTOCOL permission envelope (`type`, `key`, `headers.operation = "insert"`, `value`) and explicitly sets `contentType: 'application/json'` on the append call. |

## Cross-Cutting Answers

### 1. Are we setting content type correctly per the base protocol?

Mostly yes.

- Every reviewed JSON writer that creates or produces into a stream sets
  `application/json` on the stream handle, the producer, or both.
- `src/main.rs:545-551` correctly uses the source blob MIME type for
  one-shot blob stream creation.
- The one notable nuance is not content type mismatch but error handling around
  creation: bootstrap and host-topology treat all `create_with()` failures as
  retryable until timeout instead of surfacing `409 Conflict` immediately.

### 2. Are we using `CreateOptions` correctly?

Mostly yes.

- JSON streams are created with
  `CreateOptions::new().content_type("application/json")`, which matches the
  base protocol’s creation rules.
- Blob uploads use
  `CreateOptions::new().content_type(content_type).initial_data(...).closed(true)`,
  which is a good fit for the base protocol’s create-and-close flow.
- The misuse is not the option set itself but the surrounding retry logic in
  bootstrap and host-topology, which can obscure permanent config mismatches.

### 3. Are Fireline state events in STATE-PROTOCOL format?

Mostly yes on the shared state stream.

Aligned STATE-PROTOCOL writers:

- `crates/fireline-harness/src/trace.rs`
- `crates/fireline-harness/src/approval.rs`
- `crates/fireline-orchestration/src/child_session_edge.rs`
- `crates/fireline-tools/src/lib.rs`
- `crates/fireline-resources/src/fs_backend.rs`
- `crates/fireline-sandbox/src/stream_trace.rs`
- `packages/client/src/events.ts`

Intentionally not STATE-PROTOCOL, but still correct because they write to
dedicated non-state streams:

- `crates/fireline-host/src/bootstrap.rs` deployment discovery events
- `crates/fireline-harness/src/audit.rs` audit records
- `crates/fireline-resources/src/publisher.rs` resource events
- `crates/fireline-tools/src/peer/stream.rs` deployment discovery projection
- `crates/fireline-resources/src/mounter.rs` blob reads

Misaligned:

- `crates/fireline-sandbox/src/providers/anthropic.rs:405-409` uses invalid
  STATE-PROTOCOL operation `"append"` on the shared state stream.

### 4. Are we flushing after writes that need durability guarantees?

Often yes, but not consistently.

Good:

- approval writes
- child-session-edge writes
- host instance/spec/endpoints helpers
- deployment discovery writes
- tool descriptor writes
- fs backend writes
- resource publisher writes
- Anthropic relay writes

Gaps:

- `crates/fireline-harness/src/trace.rs:74-88`
- `crates/fireline-harness/src/audit.rs:172-175`

Those two rely on buffered producer behavior without a synchronous durability
barrier or an asynchronous error callback.

### 5. Are producer errors handled correctly?

Not everywhere.

Good:

- every site that appends and then awaits `flush()` returns or logs the flush
  error

Weak:

- `DurableStreamTracer` hot-path appends do not observe producer errors
- `AuditTracer` durable-stream sink does not observe producer errors
- no producer in production code uses the Rust client’s `on_error(...)`
  callback

### 6. Are we using the Rust client’s reconnection/backpressure features?

Only partially.

What Fireline does use:

- `error.is_retryable()` checks in long-lived readers
- explicit outer loops that rebuild readers in resource and peer projections
- producer batching via the client’s default producer implementation

What Fireline does not use:

- `LiveMode::Auto`
- producer `on_error(...)`
- producer `auto_claim(true)`
- producer `epoch(...)`
- producer `linger(...)`
- producer `max_in_flight(...)`

So Fireline is using the client at a basic level, but not the upstream
documented recovery/backpressure feature set.

## Severity Summary

- `P0`
  - invalid STATE-PROTOCOL operation in
    `crates/fireline-sandbox/src/providers/anthropic.rs:405-409`
- `P1`
  - unobserved producer failures in `crates/fireline-harness/src/trace.rs`
  - unobserved producer failures in `crates/fireline-harness/src/audit.rs`
  - `create_with()` retry loops masking terminal conflicts in bootstrap and
    host-topology
- `P2`
  - readers do not use `LiveMode::Auto`
  - producers do not opt into the Rust client’s documented recovery/backpressure
    knobs

## Conclusion

Fireline is already structurally compatible with the upstream Durable Streams
protocols:

- JSON state streams are mostly emitted in valid STATE-PROTOCOL form.
- non-state streams are generally using the base protocol correctly.
- most synchronous lifecycle writes flush before returning.

The main protocol correctness issue is narrow and concrete: the Anthropic relay
is writing non-STATE messages into the shared state stream with
`operation = "append"`. After that, the biggest risks are operational rather
than schema-level: a few buffered producer sites never surface errors, and two
stream-creation helpers treat permanent conflicts like transient failures.
