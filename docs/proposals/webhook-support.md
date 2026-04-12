# Webhook Support Using Existing Fireline Primitives

> **SUPERSEDED** by [`./durable-subscriber.md`](./durable-subscriber.md) §5.2 "Webhook Delivery Profile". Retained for historical reference only.

## TL;DR

Fireline does not need a new broker, queue, or webhook service.

The clean solution is:

- add a `webhook(...)` middleware spec in `@fireline/client`
- map it to a `webhook_forwarder` topology component
- have the Fireline host spawn a background durable-streams live reader for the runtime's state stream
- filter matching events and `POST` them to configured HTTP endpoints
- let external systems append follow-up events, such as `approval_resolved`, back into the same state stream using the existing append path

This replaces the current "inline HTTP server + `createFirelineDB()` subscription + `fetch()`" pattern in `examples/approval-workflow/index.ts` with a host-owned stream-to-webhook bridge.

## 1. Existing Primitives We Can Reuse

### Durable Streams already gives Fireline the subscription primitive

The upstream durable-streams surface already provides exactly the live-read primitive Fireline needs:

- catch-up reads from an offset
- long-poll
- live SSE readers

The approval gate already uses this pattern in `crates/fireline-harness/src/approval.rs` by reading the state stream with `LiveMode::Sse` and waiting for `approval_resolved`.

That means webhook delivery does not need a new event transport. It is just another consumer of the same durable stream.

### The Fireline host is already the always-on process

The host already owns runtime lifecycle and watches agent state. That makes it the right place to run a long-lived stream subscriber that forwards selected events outward.

This is better than the current example because the forwarding loop moves out of the application process and into the process that already persists while the runtime is alive.

### The topology pipeline is already the configuration boundary

`packages/client/src/sandbox.ts` already lowers serializable middleware specs into topology components such as:

- `audit`
- `approval_gate`
- `budget`
- `context_injection`
- `peer_mcp`

`crates/fireline-harness/src/host_topology.rs` then resolves those component specs into runtime behavior.

Webhook forwarding fits this model naturally. It is runtime composition, not an ad hoc side process.

### Resource discovery is the right precedent for named destinations

Fireline already has stream-backed publication and discovery patterns in `crates/fireline-resources/src/publisher.rs`.

That matters for webhooks because it means Fireline does not need a separate registry service just to name endpoints. A first cut can use host-local config for destination lookup, and a later cut can resolve named webhook targets through the existing discovery model instead of inventing a new control plane.

## 2. What Durable Streams Does Not Already Provide

I did not find evidence that durable-streams already has native webhook fan-out.

The official protocol and docs expose stream creation, append, catch-up reads, long-poll, and SSE live reads. The upstream repo text likewise shows SSE readers and consumers, but not server-side HTTP callback delivery.

So the right interpretation is:

- durable-streams is the transport and durability layer
- Fireline should own the webhook bridge as a host-side consumer

## 3. Recommended Approach

Add a host-resolved topology component named `webhook_forwarder`.

Its job is simple:

1. Subscribe to the runtime state stream using the same durable-streams reader pattern the approval gate already uses.
2. Filter envelopes by configured event selectors.
3. `POST` matching envelopes to a configured destination URL.
4. Retry failed deliveries with bounded backoff.
5. Advance a persisted delivery cursor only after a `2xx` response.

This is not a tracer. Tracers only see ACP request/response traffic. The approval workflow's important event, `permission_request`, is emitted to the durable state stream inside the approval gate, so a stream subscriber is the correct hook point.

This is also not a new service. It is just another host-owned consumer of a durable stream.

## 4. Delivery Model

The delivery contract should be at-least-once.

That matches the primitives Fireline already has:

- durable streams for replayable source-of-truth events
- host-owned long-lived readers
- idempotent consumers keyed by stream offset or event key

The forwarder should include these fields in every POST body:

```json
{
  "subscription": "slack-approvals",
  "streamUrl": "http://127.0.0.1:4440/streams/state:abc",
  "offset": "00000123",
  "deliveredAtMs": 1740000000000,
  "event": {
    "type": "permission",
    "key": "sess-1:req-1",
    "headers": { "operation": "insert" },
    "value": {
      "kind": "permission_request",
      "sessionId": "sess-1",
      "requestId": "req-1",
      "reason": "approval required"
    }
  }
}
```

The receiver can then dedupe on `offset` or `event.key`.

## 5. Cursor Persistence Without New Infrastructure

The forwarder should persist its last acknowledged offset using another durable stream, not a new database.

Recommended shape:

- one cursor stream per subscription, or one shared stream keyed by `{host_key}:{subscription}`
- append or update a small JSON record after each successful delivery
- on startup, rebuild the last delivered offset from that cursor stream and resume from there

This keeps the design inside existing primitives:

- source stream: Fireline state stream
- delivery state: another durable stream
- always-on worker: Fireline host

No Redis, no queue broker, no external scheduler.

## 6. API Surface

### Client middleware

Add a new middleware helper:

```ts
webhook({
  target: 'slack-approvals',
  events: ['permission_request'],
})
```

Proposed TypeScript shape:

```ts
export interface WebhookMiddleware {
  readonly kind: 'webhook'
  readonly target?: string
  readonly url?: string
  readonly events: readonly (string | { readonly type: string; readonly kind?: string })[]
}
```

Rules:

- `target` is the preferred production form
- `url` is allowed for local/dev parity with today's example
- string selectors match `value.kind` first and fall back to `type`
- object selectors are the escape hatch for exact `type`/`kind` matching

### Topology lowering

`packages/client/src/sandbox.ts` should lower that middleware to:

```ts
{
  name: 'webhook_forwarder',
  config: {
    target: 'slack-approvals',
    events: ['permission_request'],
  },
}
```

### Host-side config

The host should resolve named targets from its own config, not force secrets into the serialized runtime spec.

Recommended host config shape:

```toml
[webhooks.targets.slack-approvals]
url = "https://hooks.slack.com/services/..."
timeout_ms = 5000
max_attempts = 8
headers = { "X-Fireline-Source" = "approval-gate" }
```

This keeps secrets and long-lived endpoint configuration on the host side while preserving portable middleware specs in client code.

## 7. How It Composes with `approve()`

`approve()` and `webhook()` should compose without any special coupling because they already share the same durable state stream.

Example:

```ts
compose(
  sandbox(),
  middleware([
    approve({ scope: 'tool_calls' }),
    webhook({ target: 'slack-approvals', events: ['permission_request'] }),
  ]),
  agent(['claude-sonnet-4-6']),
)
```

Runtime flow:

1. `approval_gate` emits `permission_request` to the state stream.
2. `webhook_forwarder` sees that envelope and POSTs it to Slack, GitHub, or another automation endpoint.
3. The external system decides and appends `approval_resolved` back to the same state stream using the existing append helper or raw durable-streams append.
4. The approval gate's existing SSE wait loop observes `approval_resolved` and unblocks the prompt.

That is the important property: `approve + webhook` works because one component writes to durable state and the other subscribes to durable state. No polling loop is needed in the application process.

## 8. Clean Replacement for `examples/approval-workflow`

The ugly part of the current example is not the webhook receiver. It is the local bridge:

- subscribe to `db.collections.permissions`
- detect pending approvals in-process
- `fetch()` an HTTP endpoint manually

With `webhook_forwarder`, the example becomes cleaner:

- Fireline host owns event delivery
- the external app only needs to receive the webhook and write back `approval_resolved`
- `createFirelineDB()` remains useful for UI/state observation, but it is no longer required just to trigger outbound notifications

## 9. Why This Is the Right Cut

This proposal stays inside existing architecture:

- durable-streams SSE readers for live delivery
- Fireline host for always-on execution
- topology/middleware for serialized configuration
- durable streams again for cursor persistence
- optional resource discovery later for named target resolution

The result is a small, composable feature:

- no new infrastructure
- no polling
- no app-local forwarding loop
- no special-case approval transport

## References

- Durable Streams docs: https://durablestreams.com/
- Durable Streams protocol: https://github.com/durable-streams/durable-streams/blob/main/PROTOCOL.md
- Durable Streams state protocol: https://github.com/durable-streams/durable-streams/blob/main/packages/state/STATE-PROTOCOL.md
