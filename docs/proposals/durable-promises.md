# Durable Promises

> Status: proposal
> Date: 2026-04-12
> Scope: TypeScript workflow API, Rust workflow context, passive subscriber sugar

## TL;DR

Awakeables are the imperative projection of `DurableSubscriber::Passive`.

- They let application code write `await ctx.awakeable<T>(...)` as a durable suspension point.
- The runtime reconstructs the suspended wait by replaying the session stream after restart.
- Resolution is still just a completion envelope appended to the same durable stream.
- This proposal does not add a new substrate; it adds user-facing sugar over [durable-subscriber.md](./durable-subscriber.md).

## 1. Relationship to DurableSubscriber

Awakeable is not a second workflow engine.

It is the user-facing API for the passive mode already defined in [durable-subscriber.md](./durable-subscriber.md):

- `ctx.awakeable()` declares "wait for completion keyed by this `CompletionKey`"
- `await awakeable.promise` durably suspends the current workflow step
- `resolveAwakeable(key, value)` appends the completion envelope the passive subscriber is waiting on
- replay reconstructs the same unresolved waits from the stream and resumes them when the completion is present

Conceptually:

```text
ctx.awakeable(key) -> PassiveSubscriber keyed by CompletionKey
resolveAwakeable(key, value) -> append completion envelope
resume -> replay stream, find completion, resolve promise
```

The important boundary is unchanged:

- agent-plane stream holds the wait/completion semantics
- infrastructure-plane subscriber state holds driver bookkeeping

## 2. Canonical identity for awakeables

Awakeables do not get Fireline-minted UUIDs.

They use the same `CompletionKey` spine as `DurableSubscriber`:

- prompt-scoped awakeable: `PromptKey(SessionId, RequestId)`
- tool-scoped awakeable: `ToolKey(SessionId, ToolCallId)`
- workflow-authored step awakeable inside a single prompt: `PromptStepKey(SessionId, RequestId, StreamOffset)`

`PromptStepKey` is still not a synthetic identity. The third coordinate is the durable-streams offset of the `awakeable_waiting` event, not a Fireline counter, UUID, or hash. If awakeables require more than the current `CompletionKey` variants, the additive change is to extend that existing enum, not to create a parallel awakeable id type.

Rules:

- `SessionId`, `RequestId`, and `ToolCallId` use canonical ACP SDK types
- `StreamOffset` is a durable-stream coordinate, not an agent-graph identifier
- no awakeable key is derived from payload fingerprints, prompt text, or random tokens

## 3. The API

TypeScript:

```typescript
import type { RequestId, SessionId, ToolCallId } from '@agentclientprotocol/sdk'
import type { CompletionKey } from '@fireline/client'

interface Awakeable<T> {
  readonly key: CompletionKey
  readonly promise: Promise<T>
}

interface WorkflowContext {
  awakeable<T>(
    scope:
      | { kind: 'prompt'; sessionId: SessionId; requestId: RequestId }
      | { kind: 'tool'; sessionId: SessionId; toolCallId: ToolCallId }
      | { kind: 'step' }
  ): Awakeable<T>
}

const approval = ctx.awakeable<boolean>({
  kind: 'prompt',
  sessionId,
  requestId,
})

const allowed = await approval.promise
```

External resolution:

```typescript
import { resolveAwakeable } from '@fireline/client'

await resolveAwakeable(approval.key, true)
```

Rust:

```rust
use sacp::schema::{RequestId, SessionId, ToolCallId};

let approval = ctx.awakeable::<bool>(CompletionKey::PromptKey(session_id, request_id));
let allowed = approval.await?;
```

For `kind: 'step'`, the workflow context derives `PromptStepKey(SessionId, RequestId, StreamOffset)` from the current request and the offset of the emitted wait event. The user does not supply that coordinate manually.

## 4. Composition

Awakeables are ordinary promises from the application's perspective, so normal async composition works:

```typescript
const winner = await Promise.race([
  approval.promise,
  durableSleep({ hours: 1 }),
])
```

```typescript
const [legal, security] = await Promise.all([
  legalReview.promise,
  securityReview.promise,
])
```

This does not imply Restate-style deterministic replay of arbitrary user code.

Scope boundary:

- awakeables are durable pause points
- code between awakeables is ordinary async code
- if the process dies between two awakeables, that code may re-run on resume
- handlers must therefore remain idempotent around side effects

That is the same discipline already required by [durable-subscriber.md](./durable-subscriber.md). The imperative surface makes suspension readable; it does not turn Fireline into a general deterministic workflow VM.

## 5. Resolution sources

An awakeable may be resolved by any writer that appends the matching completion envelope:

- external caller via `resolveAwakeable(key, value)` from a dashboard, Slackbot, or webhook handler
- another subscriber that actively produces the completion envelope
- the agent itself via `agent.resolvePermission()`, which is just approval-specific awakeable resolution under the hood

The waiter does not care who resolved it. The only thing that matters is that the completion envelope matches the same `CompletionKey`.

## 6. Use cases that become trivial

Human approval:

```typescript
const approval = ctx.awakeable<boolean>({ kind: 'prompt', sessionId, requestId })
await sendSlackApproval(approval.key)
if (!(await approval.promise)) throw new Error('denied')
```

Pure-subscriber equivalent: define the passive subscriber, expose the completion endpoint, and manually connect the resumed control flow.

Multi-step saga:

```typescript
const fundsHeld = await ctx.awakeable<HoldResult>({ kind: 'step' }).promise
try { await shipOrder(fundsHeld) }
catch (err) { await releaseHold(fundsHeld); throw err }
```

Pure-subscriber equivalent: multiple domain completion envelopes plus explicit state-machine transitions.

Multi-reviewer flow:

```typescript
const legal = ctx.awakeable<boolean>({ kind: 'step' })
const security = ctx.awakeable<boolean>({ kind: 'step' })
await Promise.all([legal.promise, security.promise])
```

Pure-subscriber equivalent: two passive subscribers and a join condition over both completion keys.

External callback:

```typescript
const callback = ctx.awakeable<VendorPayload>({ kind: 'step' })
await sendVendorRequest({ resumeKey: callback.key })
const payload = await callback.promise
```

Pure-subscriber equivalent: webhook subscriber plus separate callback correlation plumbing.

Scheduled wake:

```typescript
const approval = ctx.awakeable<boolean>({ kind: 'prompt', sessionId, requestId })
const result = await Promise.race([approval.promise, durableSleep({ hours: 1 })])
```

Pure-subscriber equivalent: passive approval subscriber plus timer subscriber plus explicit join logic.

## 7. Acceptance criterion alignment

This proposal inherits the bar from [acp-canonical-identifiers.md](./acp-canonical-identifiers.md).

- `Awakeable.key` is `CompletionKey`, not a Fireline UUID
- prompt and tool awakeables are keyed only by ACP schema identifiers
- step awakeables use canonical ACP ids plus the durable-stream offset of the wait event
- `_meta.traceparent`, `_meta.tracestate`, and `_meta.baggage` propagate exactly as they already do for subscribers
- awakeable completion events live on the agent-plane stream
- subscriber retries, dead letters, and delivery bookkeeping stay in the infrastructure plane

Validation checklist:

- [ ] No awakeable API accepts or returns a synthetic UUID or hash id
- [ ] All ACP identifiers in awakeable APIs use `sacp::schema` or `@agentclientprotocol/sdk` types
- [ ] `resolveAwakeable()` resolves by `CompletionKey`, not by a Fireline-only identifier
- [ ] Prompt/tool awakeables use only canonical ACP identifiers
- [ ] Step awakeables derive their third coordinate from durable-stream offset, not a Fireline counter
- [ ] No awakeable bookkeeping leaks onto infrastructure rows or requires a bespoke lineage table

## 8. Open questions

Deterministic replay between awakeables:

- out of scope here
- that is a different proposal space closer to Restate-style workflow replay

Cross-organization awakeable resolution:

- deferred
- requires a shared durable-streams access story across trust boundaries

Timeout API shape:

- choose explicit `Promise.race([awakeable.promise, durableSleep(...)])` for the first cut
- do not add a special timeout-flavored awakeable primitive yet
- that keeps the substrate small and composes with the existing timer subscriber design

## 9. Implementation note

- no new Rust substrate is needed; this is a thin API over `DurableSubscriber::Passive`
- `ctx.awakeable()` is sugar for "declare a passive wait keyed by `CompletionKey` and return a promise bound to it"
- `resolveAwakeable()` is sugar for appending the matching completion envelope
- `agent.resolvePermission()` remains the approval-specific resolver on top of the same mechanism
- TLA+ work from [durable-subscriber.md](./durable-subscriber.md) applies unchanged; awakeable correctness is subsumed by passive-subscriber correctness

## References

- [durable-subscriber.md](./durable-subscriber.md)
- [acp-canonical-identifiers.md](./acp-canonical-identifiers.md)
- [approval-gate-correctness.md](../reviews/approval-gate-correctness.md)
- Restate human-in-the-loop pattern: https://docs.restate.dev/ai/patterns/human-in-the-loop
