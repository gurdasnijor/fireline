# Claude Agent SDK v2 — API surface verification

> **Status:** findings report, no code / interface changes recommended from this alone.
> **Produced for:** the `host-claude` satisfier lane (Step 3 / Tier E in [`../proposals/runtime-host-split.md#7-host--sandbox--orchestrator-reframe-post-stress-test`](../proposals/runtime-host-split.md)).
> **Reviews:** [`../proposals/client-primitives.md`](../proposals/client-primitives.md) §"Stress-test example — Claude Agent SDK v2 host" (lines ~653–774).
> **Fetched from:** https://code.claude.com/docs/en/agent-sdk/typescript-v2-preview on 2026-04-11.
> **Version anchor:** page header is *"TypeScript SDK V2 interface (preview)"* with the explicit warning *"The V2 interface is an **unstable preview**. APIs may change based on feedback before becoming stable."* All public entrypoints are prefixed `unstable_v2_*`. **There is no semver version number on the page** — the version is implicitly tied to the current `@anthropic-ai/claude-agent-sdk` npm package, with `unstable_v2_*` exports.

## Summary

**The existing sketch in `client-primitives.md` is written against the V1 SDK, not V2.** Three divergences are load-bearing; the other differences fall out of those three. The Host primitive interface itself (createSession / wake / status / stopSession) **does not need adjustment** — V2 maps cleanly onto it — but the illustrative code block needs a full rewrite, not field renames.

Per `runtime-host-split.md` §7 workspace:4 escalation policy: this falls on the "report back, do not auto-edit the proposal doc" side of the line. A V2-correct rewrite is provided as [§6 Appendix](#6-appendix--proposed-v2-correct-sketch) of this findings doc so the work is done if the proposal authors want to land it verbatim, but the decision is theirs.

## 1. Package and import path

| Concern | Sketch | V2 actual |
|---|---|---|
| Package name | `@anthropic-ai/agent-sdk` | **`@anthropic-ai/claude-agent-sdk`** |
| Import style | `import { query } from '@anthropic-ai/agent-sdk'` | `import { unstable_v2_createSession, unstable_v2_resumeSession, unstable_v2_prompt, type SDKMessage } from '@anthropic-ai/claude-agent-sdk'` |

The sketch's package name is a guess; the real published package is `@anthropic-ai/claude-agent-sdk`. Every V2 public symbol carries the `unstable_v2_` prefix to signal preview status. The sketch has no prefix anywhere.

## 2. Programming model — the big divergence

### V1 shape (what the sketch uses)

```ts
// Sketch, client-primitives.md:706–712
const result = query({
  prompt: pending.map(p => p.text).join('\n\n'),
  options: {
    resume: claudeSessionId,
    model: opts.model ?? 'claude-opus-4-6',
  },
})
for await (const msg of result) { /* ... */ }
```

Single `query()` call. Returns an async iterable directly. `resume` is a field under `options`. One call = one multi-turn interaction driven by whatever prompt you gave; iterate until done.

### V2 shape (what the docs actually expose)

Three top-level entrypoints, all prefixed `unstable_v2_`:

```ts
// From https://code.claude.com/docs/en/agent-sdk/typescript-v2-preview
// "API reference" section, quoted verbatim:

function unstable_v2_createSession(options: {
  model: string;
  // Additional options supported
}): SDKSession;

function unstable_v2_resumeSession(
  sessionId: string,
  options: {
    model: string;
    // Additional options supported
  }
): SDKSession;

function unstable_v2_prompt(
  prompt: string,
  options: {
    model: string;
    // Additional options supported
  }
): Promise<SDKResultMessage>;

interface SDKSession {
  readonly sessionId: string;
  send(message: string | SDKUserMessage): Promise<void>;
  stream(): AsyncGenerator<SDKMessage, void>;
  close(): void;
}
```

**Key shape differences:**

1. **`createSession` / `resumeSession` are separate entrypoints**, not options on a single function. `resume` is not a field — it's a different function call (`unstable_v2_resumeSession(sessionId, options)`).
2. **The session is a live object**, not a stream. You hold an `SDKSession` across turns and call `send()` + `stream()` on it repeatedly.
3. **`send()` and `stream()` are distinct calls per turn.** `send()` dispatches the user message (returns `Promise<void>`); `stream()` yields the agent response for *that turn* via an `AsyncGenerator<SDKMessage, void>`. One turn = one send→stream cycle. Multi-turn means calling `send()` again on the same session.
4. **The session has its own `sessionId` property** (`readonly sessionId: string`) — you can read it off the `SDKSession` object directly without waiting for a streamed message. The sketch's "wait for `msg.type === 'system' && msg.subtype === 'init'` then grab `msg.session_id`" dance is unnecessary in V2. Every streamed `SDKMessage` also carries `session_id`, per the example: `sessionId = msg.session_id`.
5. **`session.close()` exists** for per-session cleanup, plus a TypeScript 5.2+ `await using` form for automatic cleanup. The sketch says *"Claude has no explicit session delete in the current SDK"* — half-right: there's no persistent delete verb documented (the docs don't clarify whether `close()` deletes server-side state or just disconnects the local handle), but there IS a local `close()` method.

### This is classified as MAJOR per workspace:4's gate

Workspace:4's escalation rule: *"If the divergences are MAJOR (fundamentally different programming model, different streaming semantics, different resume story), DO NOT edit the doc."* The sketch calls `query()` once and iterates; V2 holds an `SDKSession` across turns and interleaves `send()`/`stream()`. That's a different programming model. Reporting, not auto-editing.

## 3. But: the Host primitive interface still works unchanged

This is the important negative finding. The `Host` trait's four methods — `createSession`, `wake`, `status`, `stopSession` — accommodate the V2 programming model cleanly. Specifically:

| Host verb | V2 satisfier maps to |
|---|---|
| `createSession(spec)` | `unstable_v2_createSession({ model })`, stash the resulting `SDKSession` in an in-memory `Map<handle.id, SDKSession>`, return a `SessionHandle` using a satisfier-generated id |
| `wake(handle)` | Drain pending inputs from our durable stream; `session.send(concat)`; iterate `session.stream()` mirroring messages to our stream; return `WakeOutcome::Advanced { steps }` |
| `status(handle)` | Peek the pending inputs registry (same as sketch) — this works verbatim |
| `stopSession(handle)` | `session.close()`; delete from the in-memory map; append `state: 'stopped'` to our stream |

The only non-obvious consequence: **a V2-based ClaudeHost has process-lifetime-sticky state** (the in-memory `Map<handle.id, SDKSession>`). If the Node process running the host crashes, the next process has to *reconstruct* the `SDKSession` via `unstable_v2_resumeSession(claudeSessionId, { model })` using the `claudeSessionId` it persisted to the durable stream on first wake. The V1 sketch avoided this because it called `query({ resume: claudeSessionId })` fresh on every wake — but that was a property of V1's stateless-per-call model, not a requirement of the Host primitive.

The lazy-reconstruction pattern (check the in-memory map; if absent, `unstable_v2_resumeSession`) fits inside `wake()` cleanly and is symmetric to how `FirelineHost` transparently reconnects to a running runtime after control-plane restart via the session registry. **No Host interface change required.**

## 4. Findings by category (per workspace:4's step 3)

### 4a. What matches the sketch

- Streaming response shape: `session.stream()` returns an async generator, same as the sketch's `for await (const msg of result)` loop.
- Assistant message structure: verbatim confirmation from the V2 docs' example —
  ```ts
  if (msg.type === "assistant") {
    const text = msg.message.content
      .filter((block) => block.type === "text")
      .map((block) => block.text)
      .join("");
  }
  ```
  This exactly matches the sketch's assumption about assistant messages.
- Per-message `session_id`: V2 docs' example uses `sessionId = msg.session_id` in a loop, confirming every yielded `SDKMessage` carries a `session_id` field. The sketch's `msg.session_id` access is valid.
- `model: string` option on session creation: confirmed as the only documented required field.

### 4b. What's different (renames, reshaping)

- `@anthropic-ai/agent-sdk` → `@anthropic-ai/claude-agent-sdk`
- `query({ prompt, options: { resume, model } })` → `unstable_v2_resumeSession(sessionId, { model })` + `session.send(prompt)` + `session.stream()` (three separate operations)
- `msg.type === 'system' && msg.subtype === 'init'` → `session.sessionId` (directly on the session object, no init-message scan needed)
- *"No session delete API"* → `session.close()` exists (local cleanup; server-side semantics undocumented)

### 4c. What's missing from the sketch (V2 has it, sketch doesn't use it)

- The V2 `SDKSession` object model itself — a live, long-lived handle with `send`/`stream`/`close`.
- `unstable_v2_prompt(prompt, options)` → `Promise<SDKResultMessage>` — a one-shot convenience function for fire-and-forget single-turn queries. Not needed for the ClaudeHost satisfier (which is inherently multi-turn), but worth noting.
- `await using` (TS 5.2+) automatic resource management. The satisfier could use this internally if it were willing to require TS 5.2+.
- `type SDKMessage` as a publicly importable type, used in the V2 example for the `getAssistantText(msg: SDKMessage)` helper.

### 4d. What's extra in the sketch (V2 doesn't have it or handles it differently)

- **`anthropicApiKey: string` required option.** The V2 docs do not show an API key in `createSession`'s options. The SDK almost certainly resolves it from the `ANTHROPIC_API_KEY` environment variable (that's the SDK convention), but this is not verified from the V2 docs page alone — the options type is documented as `{ model: string; /* Additional options supported */ }` with the "additional options" not enumerated. The sketch's explicit `anthropicApiKey` field may not match the real SDK contract. **Flag as unverified.**
- **The init-message scan for `session_id`.** Obsolete in V2 — read `session.sessionId` directly.
- **`StreamProducer` and `PendingInputRegistry` as required options.** These are Fireline-side abstractions, not SDK things — not a divergence, but worth naming: these are the sketch's way of saying "the satisfier needs a way to mirror messages into our durable stream and a way to read pending inputs." Both still belong on the satisfier regardless of SDK version.

## 5. What I could NOT verify from the fetched doc

- **Tool execution model.** The V2 preview page says *"Session forking ... only available in the V1 SDK"* but does not describe how tool calls are handled in V2 — whether tool_use events come back to the caller for external execution with tool_result streamed back, or whether Claude's managed sandbox runs them transparently. The stress-test example's wake loop iterates `session.stream()` and mirrors everything; it doesn't explicitly handle `tool_use` vs `tool_result`. **This gap matters for the §7 Sandbox-delegation story** (can a ClaudeHost delegate tool execution to a `MicrosandboxSandbox`?) and should be investigated separately before wiring Step 3. Possible sources: the V1 docs (linked from the V2 preview page), `github.com/anthropics/claude-agent-sdk-demos`, or the package source.
- **Authentication mechanism.** Options type is documented as `{ model: string; /* Additional options supported */ }`. The "Additional options supported" comment deliberately punts. API key is presumably env-var-based but not confirmed.
- **Session lifecycle beyond `close()`.** The docs describe `close()` as *"Session closes automatically when the block exits"* in the `await using` form and as manual cleanup otherwise. They do NOT specify whether closing destroys server-side session state, whether there's a server-side TTL, or whether a closed-and-then-resumed-with-the-same-session-id flow is allowed. **Important for the `stopSession` → later `resumeSession` round-trip the sketch's `wake()` assumes**; file as a follow-up verification item.
- **Error / failure messages.** The sketch doesn't handle errors; the V2 docs' examples also don't. Production satisfiers will need error-kind detection in the `stream()` loop; kind list unknown from the docs alone.

## 6. Appendix — proposed V2-correct sketch

If the proposal authors accept the V2 rewrite, the replacement for the code block at `client-primitives.md:658–762` would be:

```ts
// @fireline/client/host-claude (proposed v2-correct rewrite)
import {
  unstable_v2_createSession,
  unstable_v2_resumeSession,
  type SDKMessage,
  type SDKSession,
} from '@anthropic-ai/claude-agent-sdk'
import type { Host, SessionHandle, WakeOutcome } from '@fireline/client/host'

export function createClaudeHost(opts: {
  readonly model?: string
  readonly stateProducer: StreamProducer        // mirrors Claude output into our durable stream
  readonly pendingInputs: PendingInputRegistry  // user's way of saying "process this next"
}): Host {
  // Process-lifetime cache of live SDKSession handles. Rebuilt lazily
  // on wake() via unstable_v2_resumeSession when a handle is missing
  // (e.g. after a process restart).
  const live = new Map<string, SDKSession>()
  const model = opts.model ?? 'claude-opus-4-6'

  async function acquire(handleId: string): Promise<SDKSession> {
    const existing = live.get(handleId)
    if (existing) return existing

    // Restore from the durable stream — we stashed the Claude
    // sessionId on the first successful wake.
    const stashed = await opts.stateProducer.readOne({ type: 'session', key: handleId })
    const claudeSessionId: string | undefined = stashed?.value?.claudeSessionId
    const sdkSession = claudeSessionId
      ? unstable_v2_resumeSession(claudeSessionId, { model })
      : unstable_v2_createSession({ model })
    live.set(handleId, sdkSession)
    return sdkSession
  }

  return {
    async createSession(spec) {
      const id = `claude:${crypto.randomUUID()}`
      const sdkSession = unstable_v2_createSession({ model: spec.model ?? model })
      live.set(id, sdkSession)
      await opts.stateProducer.append({
        type: 'session',
        key: id,
        headers: { operation: 'insert' },
        value: {
          sessionId: id,
          host: 'claude',
          model: spec.model ?? model,
          state: 'created',
          createdAt: Date.now(),
          claudeSessionId: sdkSession.sessionId,
        },
      })
      if (spec.initialPrompt) {
        await opts.pendingInputs.push(id, { kind: 'prompt', text: spec.initialPrompt })
      }
      return { id, kind: 'claude' }
    },

    async wake(handle): Promise<WakeOutcome> {
      const pending = await opts.pendingInputs.drain(handle.id)
      if (pending.length === 0) return { kind: 'noop' }

      const sdkSession = await acquire(handle.id)
      const prompt = pending.map(p => p.text).join('\n\n')

      await sdkSession.send(prompt)

      let steps = 0
      for await (const msg of sdkSession.stream()) {
        await opts.stateProducer.append({
          type: 'claude_message',
          key: `${handle.id}:${msg.session_id}:${steps}`,
          headers: { operation: 'insert' },
          value: msg,
        })
        steps += 1
      }

      await opts.stateProducer.append({
        type: 'session',
        key: handle.id,
        headers: { operation: 'update' },
        value: { state: 'running', claudeSessionId: sdkSession.sessionId },
      })
      await opts.stateProducer.append({
        type: 'pending_resolved',
        key: handle.id,
        headers: { operation: 'insert' },
        value: { count: pending.length, resolvedAt: Date.now() },
      })

      return { kind: 'advanced', steps }
    },

    async status(handle) {
      const pending = await opts.pendingInputs.peek(handle.id)
      return pending.length > 0 ? { kind: 'needs_wake' } : { kind: 'idle' }
    },

    async stopSession(handle) {
      const sdkSession = live.get(handle.id)
      sdkSession?.close()
      live.delete(handle.id)
      await opts.stateProducer.append({
        type: 'session',
        key: handle.id,
        headers: { operation: 'update' },
        value: { state: 'stopped' },
      })
      // V2 has session.close() but no documented server-side delete
      // verb. State persists server-side for later unstable_v2_resumeSession
      // as far as the docs reveal.
    },
  }
}
```

**Shape notes on the rewrite:**

- `createSession` *now actually calls* `unstable_v2_createSession` eagerly (the V1 sketch deferred to `wake()`). This is possible in V2 because session creation is synchronous — no round-trip required. If eager creation is undesirable (e.g., the user wants to reserve an id without burning an SDK session), a `lazy` mode is trivial to add.
- The `acquire(handleId)` helper is new and central: it's what bridges "in-memory live session" with "persistent session id in our durable stream." Any process restart hits the `unstable_v2_resumeSession` branch. This pattern is symmetric to how `FirelineHost` would transparently reconnect to a live runtime via `RuntimeRegistry` after a control-plane restart.
- `anthropicApiKey` is dropped from the options signature because V2 doesn't document it on the type. If the SDK turns out to require explicit auth-passing, it'll need to be added back — flagged in §5 as unverified.
- `msg.type === 'system' && msg.subtype === 'init'` scan is gone — replaced by `sdkSession.sessionId` direct access.
- `stopSession` calls `sdkSession.close()` then removes from the in-memory map.

## 7. Recommended next steps

1. **Proposal authors decide** whether to accept the §6 appendix as a patch to `client-primitives.md:658–762` or keep the V1 sketch with an added "uses V1 shape; see `claude-agent-sdk-v2-findings.md` for V2" note. This is a documentation-hygiene call, not an architectural one.
2. **Before Step 3 (Tier E) writes code**, verify the three open items from §5:
   - How does V2 surface tool_use / tool_result events? (Needed to decide whether `ClaudeHost` can delegate to `MicrosandboxSandbox` or whether Claude's managed tool execution is opaque.)
   - Is API key env-var-only or is there an options field? (Needed for ClaudeHost's construction signature.)
   - Does `close()` destroy server-side session state, or can a closed-and-later-resumed flow work? (Needed for process-restart correctness.)
   The V1 docs page (linked from V2 preview as the full API reference) and `github.com/anthropics/claude-agent-sdk-demos/tree/main/hello-world-v2` are both promising sources.
3. **The Host primitive interface does not need adjustment** — the V2 shape fits cleanly inside the existing four-verb contract. Step 3 can proceed against the current `@fireline/client/host` interfaces (workspace:11's Tier 2, commit `0284761`) once the §5 open items are resolved.

---

**Document pinning:** if `@anthropic-ai/claude-agent-sdk` publishes a version tag after this fetch (2026-04-11), re-verify the `unstable_v2_*` surface against that release before acting on the §6 rewrite. The preview warning on the page explicitly says APIs may change before stabilization.
