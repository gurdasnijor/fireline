# Fireline Demo Walkthrough — 2026-04-12

> Authoritative click-by-click script for the 2026-04-12 demo of the Fireline substrate. Pair with [`./demo-runbook.md`](./demo-runbook.md) for environment bring-up and fallback steps.
>
> **Companion references:**
> - [`./proposals/client-primitives.md`](./proposals/client-primitives.md) — the TypeScript primitive surface (`Host`, `Sandbox`, `Orchestrator`) this demo is built on.
> - [`./proposals/runtime-host-split.md`](./proposals/runtime-host-split.md) §7 — Host / Sandbox taxonomy on the Rust side.
> - [`./explorations/managed-agents-mapping.md`](./explorations/managed-agents-mapping.md) — the six-primitive source of truth.
> - `verification/spec/managed_agents.tla` — the TLA spec whose invariants the demo narrates live.

## 1. What we're showing

Fireline is a substrate for managed agents. Its surface is the six Anthropic managed-agent primitives — **Session**, **Orchestration**, **Harness**, **Sandbox**, **Resources**, **Tools** — and nothing else. Every UI element on screen and every log line you'll see maps to one of those six primitives through the `Host` / `Sandbox` / `Orchestrator` trait trio introduced in [`./proposals/client-primitives.md`](./proposals/client-primitives.md).

The demo's narrative beat is **formal-first design**. Two vocabulary passes landed on the TLA spec before any of the current code existed:

- **Level 1 alignment** (commit `7c990d1`, *"Rename TLA Resume* vocabulary to Wake* for primitive alignment"*) renamed every `Resume*` action, variable, and invariant in `verification/spec/managed_agents.tla` to `Wake*` so the formal model speaks the same verb the TypeScript `Host.wake(handle)` primitive speaks.
- **Level 2 alignment** (commit `a0bfe8b`, *"Split host and sandbox in managed-agents TLA spec"*) split the TLA spec's `Host` concept into explicit `Host` (session lifecycle) and `Sandbox` (tool execution) primitives, mirroring the code-side split in `crates/fireline-conductor/src/primitives/`.

We then let the **crate layout inherit the structure**: the target Rust workspace in [`./proposals/crate-restructure-manifest.md`](./proposals/crate-restructure-manifest.md) has nine crates, each aligned 1:1 with a primitive from the taxonomy above. The formal spec and the crate boundaries match by construction. That's the story: *formal model first, primitive taxonomy second, crates and code third, browser UI fourth — each layer inherits structure from the one above it*.

What you'll actually see on screen: the Fireline browser harness (Vite-hosted React app at `http://localhost:5173`) driving the live `Host` primitive end-to-end. The left pane is the ACP session harness (runtime controls + events log + prompt input + inspector). The right pane is the `@fireline/state` durable-stream explorer, showing the browser observing the same events the runtime is writing.

The agent running inside the runtime is **`fireline-testy-load`** — a minimal ACP agent that supports `session/load` and echoes `"Hello, world!"` for any prompt. The point of using testy-load (not a real model) is to keep the demo deterministic: every assistant response is exactly `Hello, world!`, and every state stream row appears on a predictable schedule.

## 2. The demo, click by click

> **Prerequisite:** the runbook's startup sequence has been executed and the browser tab at `http://localhost:5173` shows *"Fireline Browser Harness"* in the header with the status pill reading `disconnected`. The bottom event log is empty.

### 2.1 Launch a runtime

**Action:** Select **"Fireline Testy Load (command)"** in the agent dropdown. Click **"Launch Agent"**.

**On screen:**
- `runtimePending` flips true (the button briefly disables).
- The events log appends a `runtime_launch` event whose payload includes the `handle` (`{ id: "runtime:<uuid>", kind: "fireline" }`) and a `status` of `{ kind: "running" }`.
- The right-hand inspector card **"Current Session"** shows:
  - `handleId` populates with `runtime:<uuid>` (monospace).
  - `sessionStatus` reads `running`.
  - `statePlane` flips from `idle until runtime is ready` to the DB fields.
- The right-hand **State Explorer** panel flips from *"Idle until a runtime is ready"* to showing the five tabs `sessions / turns / edges / chunks / connections`. The `sessions` tab is the default; it's empty because no ACP session has been opened yet, but the preload has completed against `http://localhost:5173/v1/stream/fireline-harness-state` — durable-streams is HTTP+SSE, not WebSocket — which vite's `/v1` proxy forwards to `127.0.0.1:4437`.

**Primitive path:** the button calls `host.provision({ agentCommand, metadata })` via `createFirelineHost` from `@fireline/client/host-fireline`. The satisfier POSTs to `/cp/v1/runtimes` with `{ provider: 'local', host: '127.0.0.1', port: 0, ... }` — Vite proxies `/cp` to `http://127.0.0.1:4440` after stripping the prefix, so this hits the `fireline-control-plane` binary on port 4440. The control plane spawns a `fireline` binary child process which binds on an OS-assigned port for its ACP WebSocket and state-stream routes (the vite proxy config pins `/acp` and `/v1` to `127.0.0.1:4437`, so the dev environment relies on the spawned runtime landing there — see the runbook port table for the caveats). The control plane waits for the runtime to advertise `Ready` and returns a `RuntimeDescriptor` to the browser. `createFirelineHost` wraps the `runtimeKey` and the descriptor's `acp` + `state` endpoints into a `HostHandle { id, kind: 'fireline', acp, state }` per the `Host.provision` contract in [`./proposals/client-primitives.md`](./proposals/client-primitives.md) §Module 2. (The `provision` verb is the post-`37db346` rename of the old `createSession` verb — the Host primitive hands out a *runtime*, and ACP sessions live inside it via `session/new` on `handle.acp.url`.)

**TLA tie-in:** at this point `runtimeIndex[runtime_key].status = "ready"` in the spec's state, and `ProvisionReturnsReachableRuntime` (at `verification/spec/managed_agents.tla:814`) says "ready runtimes are reachable" — which is exactly what the browser proves by opening a WebSocket to the same runtime in step 2.2.

### 2.2 Open an ACP session

**Action:** Click **"New Session"**.

**On screen:**
- The events log appends a `connection` event `{ mode: 'new', url: 'ws://localhost:5173/acp' }`.
- The status pill flips `disconnected → connecting → connected`.
- A `session_new` event is appended with the ACP `sessionId` returned by `session/new`. The `sessionId` code label at the top of the harness updates from `no session` to `session sess_<id>`.
- In the right pane's State Explorer `sessions` tab, a new row appears with that `sessionId`, `runtimeKey`, and `state: active`. **This is the crucial moment for the Session primitive narrative**: the browser is reading from the durable stream, not from the API response. The row showed up because the runtime's `DurableStreamTracer` projected the `session/new` ACP event into a `session` envelope on the shared state stream (`fireline-harness-state`), and `@fireline/state`'s `createFirelineDB` picked it up through its subscriber loop.

**Primitive path:** `WebSocket → ws://localhost:5173/acp → vite proxy (ws enabled) → 127.0.0.1:4437/acp` — which lands on the runtime's ACP axum router. The `ClientSideConnection` from `@agentclientprotocol/sdk` speaks the ACP protocol: `initialize` → `newSession({ cwd: '/', mcpServers: [] })`. The ACP `session/new` call is intercepted by the runtime's conductor proxy chain (currently a minimal topology with `peer_mcp`). The new session id is echoed back, and every ACP frame thereafter is traced to the shared stream.

**TLA tie-in:** `SessionAppendOnly` (at `verification/spec/managed_agents.tla:755`) says session logs are append-only with strict prefix preservation. `SessionScopedIdempotentAppend` (line 769) says producer-tuple dedupe is enforced. The row you just saw in the state explorer is a witness to the first of those invariants — the fact that the subscribed state explorer never regresses the session list is a live proof of append-only.

### 2.3 Send a prompt — Harness + Session in motion

**Action:** Type `hi from the demo` into the prompt input. Click **"Send"**.

**On screen:**
- Events log appends in order:
  1. `user_prompt { text: 'hi from the demo' }`
  2. One or more `session_update` events carrying the agent's `MessageContentBlock`s — testy-load emits a plain-text content block with body `"Hello, world!"`.
  3. `prompt_response` with the final `PromptResponse` payload (stopReason, etc.).
- The State Explorer `turns` tab (click it) shows a new `prompt_turn` row: `state: active → completed`, `text: 'hi from the demo'`, and a `stopReason`. Under `chunks` you'll see one or more chunk rows as the content streamed through.
- The `sessionStatus` stays `running` throughout.

**Primitive path:** `connection.prompt({ sessionId, prompt: [{ type: 'text', text }] })` over the ACP WebSocket → the runtime's conductor receives the prompt → the `fireline-testy-load` child process produces the response. Every visible effect on the way — the `session/prompt` request, the `session/update` notifications, and the final response — passes through the `DurableStreamTracer`'s `WriteEvent` impl and lands in the shared stream as `prompt_turn`, `chunk`, and `session_update` rows. The browser's `@fireline/state` collections re-render the affected panels via TanStack DB's live query.

**TLA tie-ins:**
- `HarnessEveryEffectLogged` (`verification/spec/managed_agents.tla:776`) — *"every visible effect lands in the session log"* — you just watched this live: the prompt text you typed became a `prompt_turn` row on the stream within a single frame of the response arriving. **Point at the state explorer** and say: *"this is the Harness primitive's core invariant. The browser sees the Harness's effect log through the same durable substrate the runtime writes to, with no shared in-memory state."*
- `HarnessAppendOrderStable` (line 783) — the order in which you see events in the stream is the order in which the agent emitted them. The live-query never reorders.

### 2.4 Reconnect + load — Session durability

**Action:** Click **"Disconnect"**. The status pill flips back to `disconnected`; the event log's latest entries show the WebSocket close. Then click **"Reconnect + Load"**.

**On screen:**
- The client opens a fresh WebSocket to `/acp`, issues `initialize`, then calls `session/load` (not `session/new`) with the same `sessionId` it captured in step 2.2.
- Events log: `connection { mode: 'load' }`, followed by a `session_load` event, followed by replays of the session-update notifications the runtime reconstructs from its own session record.
- The State Explorer side doesn't flicker — the rows from step 2.3 are still there, because they're stored on the durable stream, not in the runtime process's RAM.

**Primitive path:** the browser held onto `sessionId` across disconnect (`preserveSessionId: true` in the disconnect options). On reconnect, `ClientSideConnection.loadSession({ sessionId, cwd, mcpServers })` dispatches ACP's `session/load` method — the runtime's `LoadCoordinatorComponent` receives it, looks up the existing session in its in-memory session index (which was materialized from the shared state stream at bootstrap), and replays the session's pending updates to the new client. See [`./state/session-load.md`](./state/session-load.md) for the full protocol contract.

**TLA tie-in: `SessionDurableAcrossRuntimeDeath`** (`verification/spec/managed_agents.tla:763`) is the marquee invariant for this beat. The spec clause is:

```
SessionDurableAcrossRuntimeDeath ==
  \A rk \in RuntimeKeys :
    runtimeIndex[rk].status = "stopped" =>
      \A s \in Sessions :
        IsPrefix(stopSnapshot[rk][s], sessionLog[s])
```

Plain English: *"if a runtime is stopped, the session log snapshot taken at stop time must be a prefix of the current session log"* — i.e., the stream never loses events across a runtime lifecycle boundary. The ACP disconnect/reconnect demo above doesn't actually stop the runtime — it only drops the browser's WebSocket — but it's the visual proof of *the same property* at the ACP-session layer: the session is a durable record, not a transient connection.

## 3. Resource discovery — the durable stream as a discovery plane

**This beat shows the Resources primitive crossing a Host boundary.** A resource published on one Host becomes mountable on a different Host through the shared durable-streams service — no sidecar file transfer, no S3, no operator-configured shared volume.

### Pre-demo setup (run once before the demo)

Before the demo, publish a local directory as a discoverable resource on the shared stream. This is a one-time CLI step that seeds the `resources:tenant-demo` stream with a `resource_published` envelope:

```sh
fireline publish-resource \
  --durable-streams-url "$DURABLE_STREAMS_URL" \
  --tenant demo \
  --resource-id workspace-foo \
  --source ~/projects/foo
```

This reads `~/projects/foo`, chunks its contents into blob events on the `resources:tenant-demo` stream, and emits a `resource_published` envelope containing the `ResourceRef { kind: 'durable_stream_blob', stream: 'resources:tenant-demo', key: 'workspace-foo' }` and the tree manifest. Any Host subscribed to that tenant stream can now discover and mount the resource.

> **TODO(demo-review):** Confirm whether `fireline publish-resource` is a shipped CLI subcommand at demo time. If not yet landed, either (a) use a raw `@durable-streams` producer script to emit the envelope manually (the ResourcePublisher trait at `crates/fireline-resources` specifies the shape), or (b) skip this beat and talk through the architecture on a slide. The resource-discovery proposal ([`./proposals/resource-discovery.md`](./proposals/resource-discovery.md)) specifies the full flow.

### The demo step

**Action:** After completing the Wake beat in §4, point at the State Explorer's **sessions** tab. Explain:

> "The state stream you've been watching is one of several streams the durable-streams service hosts. Another stream — `resources:tenant-demo` — carries resource-discovery events. Before the demo I published a local directory to that stream using `fireline publish-resource`. Any Host sharing this tenant can now mount it."

Now provision a second runtime (or use the existing one if it was provisioned with a `ProvisionSpec` that references the published resource):

> "When this Host provisions a runtime, its `DurableStreamMounter` reads the `resources:tenant-demo` stream, finds the `resource_published` envelope for `workspace-foo`, downloads the blob chunks from the same stream, materializes them to a tmpfs, and bind-mounts them into the sandbox. The agent inside sees `/workspace/foo` as a normal directory — it has no idea the bytes arrived from a durable stream rather than a local path."

**What to say (the punchline):**

> "This is the same durable-streams service that carries session state and host-discovery events. **One service, three discovery surfaces**: sessions, hosts, resources. No separate file service, no artifact registry, no operator-configured volume shares. Publish to the stream, discover from the stream, mount from the stream. That's the Resources primitive implemented as a stream-backed registry."

**TLA tie-in:** `ResourcePublishedIsEventuallyDiscoverable` from `verification/spec/deployment_discovery.tla` — any reader that has replayed past the `resource_published` event observes the resource in its `ResourceIndex`. `SourceRefIsImmutableAfterPublish` — once published, the backing `source_ref` never changes; updates only touch metadata.

---

## 4. The Wake moment — the single orchestration verb

**This is the demo's money beat.** Pause here.

**Action:** Click **"Wake"** (the cyan button near the top bar, right of "Disconnect").

**On screen:**
- An event appends to the log: `wake { kind: 'noop' }`. That's the whole payload.
- Nothing else changes. `sessionStatus` stays `running`, `handleId` is unchanged, no new rows on the state explorer, no log chatter from the runtime.

**What to say:**

> "Wake is the single orchestration verb in the Fireline substrate. It's the verb Anthropic's managed-agents post calls `wake(session_id) → void` and says is satisfiable by *'any scheduler that can call a function with an ID and retry on failure'*. We took that literally: `Host.wake(handle)` is **idempotent, retry-safe, and returns a discriminated outcome**. When you call it on a runtime that's already ready — like this one — the outcome is `noop`, and nothing happens. That's not a bug; it's the spec. The formal model requires it."

Point at `verification/spec/managed_agents.tla:789`:

```
WakeOnReadyIsNoop ==
  lastWake.valid /\ lastWake.beforeStatus = "ready" =>
    /\ lastWake.createdNew = FALSE
    /\ lastWake.afterRuntimeId = lastWake.beforeRuntimeId
```

> "That invariant is enforced by the Rust `createFirelineHost` satisfier at `packages/client/src/host-fireline/client.ts:74-76`: it checks `descriptor.status === 'ready'` and returns `{ kind: 'noop' }` before doing any work. Click the button again — same result. Click it a third time — same. **Idempotent.** The TLA model-checker verifies this invariant holds across every interleaving of concurrent wake calls — see also `ConcurrentWakeSingleWinner` at line 794."

Then walk to the complementary invariant:

> "Wake on a **stopped** runtime is the other half of the primitive. The spec says:"

```
WakeOnStoppedChangesRuntimeId ==
  lastWake.valid /\ lastWake.createdNew =>
    /\ lastWake.beforeStatus = "stopped"
    /\ lastWake.afterRuntimeId # lastWake.beforeRuntimeId
    /\ runtimeIndex[lastWake.runtimeKey].runtimeId = lastWake.afterRuntimeId
```

> "Plain English: *'a successful wake that created a new runtime can only have happened from a stopped starting state, and the new runtime_id is different from whatever was there before — but the runtime_key is unchanged, so all the sessions bound to that key travel across the wake boundary'*. Combined with `WakeOnStoppedPreservesSessionBinding` at line 821, this is what makes wake a **single orchestration verb that covers both the trivial case and the recovery case**, not two verbs."

> **TODO(demo-review):** The current `createFirelineHost.wake` implementation at `packages/client/src/host-fireline/client.ts:78-80` returns `{ kind: 'blocked', reason: { kind: 'require_approval', scope: 'all' } }` when the runtime is `stopped` or `broken`, **not** a new runtime via `POST /v1/runtimes`. The `WakeOnStoppedChangesRuntimeId` beat is therefore a **spec-level demonstration**, not a click-through beat. If asked *"can you click Wake on a stopped runtime and show it coming back?"* the honest answer is *"the TS satisfier currently gates that path behind a policy decision; the Rust `fireline::orchestration::resume` helper does the full recovery composition, and slice 16 will close the loop in the TS layer. The TLA invariant is satisfied by the spec; the impl is 90% there."* Decide before demo whether to (a) show only the `WakeOnReadyIsNoop` beat live and talk through the stopped case on the slide, or (b) extend `createFirelineHost.wake` to do the cold-start composition before demo day.

## 5. The state explorer — `@fireline/state` as the universal read surface

**Action:** Click through all five tabs on the right pane: **sessions**, **turns**, **edges**, **chunks**, **connections**.

**On screen:** each tab renders a live `useLiveQuery` against a different `@fireline/state` collection. The rows you see are materialized by TanStack DB from a durable-streams subscription to `http://localhost:5173/v1/stream/fireline-harness-state` (HTTP+SSE; vite proxies `/v1` to the runtime's embedded durable-streams server on `127.0.0.1:4437`).

**What to say:**

> "Every tab here is a TanStack-DB live collection defined in `packages/state/src/schema.ts` — sessions, prompt_turns, child_session_edges, chunks, connections. The browser has no API calls against the runtime for these views; it subscribes to the durable stream directly and materializes views locally. **The browser sees the substrate as the source of truth.** If I kill the runtime right now" — *(don't actually do this during demo, just gesture)* — "these collections would stay populated because the stream is durable. If I restart the runtime and it reattaches to the same stream, the collections would automatically receive any new rows — no reload, no reconnection logic on the consumer side."

> "This is the **Session primitive** from Anthropic's managed-agents taxonomy, wired through to the browser without a translation layer. Clients don't implement a client-side Session interface. They consume `@fireline/state` collections directly. See [`proposals/client-primitives.md:430`](./proposals/client-primitives.md#module-4-fireline-state-existing-package--the-session-read-surface) (Module 4) for why this was the right call — the v1 proposal had a `Session` interface with `getEvents / emitEvent / getPendingEvents`, and v2 **deleted all three** because `@fireline/state` already did the read side and there is no client-side emit verb."

**Primitive path:** TanStack DB live query → `createFirelineDB({ stateStreamUrl })` → `@durable-streams/state` subscription → `GET http://localhost:5173/v1/stream/fireline-harness-state?live=sse` → vite proxy → `127.0.0.1:4437/v1/stream/...` → the durable-streams-server embedded inside the `fireline` binary (via `stream_host.rs`) → SSE back out through the same chain. The runtime itself writes into the same stream via its `DurableStreamTracer`, so there's a clean loop: *runtime projects ACP events → durable stream → @fireline/state materializes → TanStack DB renders → React reconciles*. **Same loop the `fireline-dashboard` TUI binary would use.**

## 6. Fallback stories — if X breaks, say Y

### 5a. Control plane fails to start

**Symptom:** terminal shows `[control-plane] ...` spam followed by an error like `timed out waiting for control plane to become ready` from `dev-server.mjs`, and the browser tab loads `http://localhost:5173` but clicking **"Launch Agent"** yields an error in the event log like `Failed to fetch` or `HTTP 502` on `/cp/v1/runtimes`.

**Say:** *"The control plane is the one component of the demo that's a separate process outside Vite. It's a Rust binary at `target/debug/fireline-control-plane` spawned by the dev server on port 4440. We're going to switch to the runbook's fallback — run it by hand."* Then go to [`./demo-runbook.md`](./demo-runbook.md) §"Known issues — 5a: control plane refuses to start" for the hand-start invocation.

**Narrative recovery:** *"While that comes up — the important thing to internalize is that the control plane is implementing the `POST /v1/runtimes` half of the `Host` primitive's contract. The browser is host-agnostic; any second satisfier (a remote hosted-API host, a stub for testing, whatever lands after demo day) would drive the same four-verb surface. The contract is `provision / wake / status / stop` on the `Host` trait in `@fireline/client/host`, not any particular backend."*

### 5b. Runtime boots, but prompts 404

**Symptom:** **"Launch Agent"** succeeds (`runtime_launch` event shows, `sessionStatus: running`), **"New Session"** succeeds (`session_new` event with a real `sessionId`), but sending a prompt produces a `prompt_response` event with an error payload or a `session_update` storm that ends in a non-text content block.

**Cause most likely:** the `fireline-testy-load` binary wasn't actually rebuilt this session and is missing a `session/prompt` handler fix, OR the agent catalog returned a stale `agentCommand`.

**Say:** *"This is an interesting window into how the substrate draws the line between 'host did its job' and 'agent did its job'. The `Host` primitive is green — we have a ready runtime, a live session, and a working WebSocket. The failure is inside the agent process — which is a deliberate design boundary. Fireline owns session lifecycle, event durability, and the proxy chain. The agent owns the actual conversation."*

**Narrative recovery:** switch to pointing at the **state explorer `chunks` tab** and say: *"Even when the prompt itself failed, you can see the chunks and turns that did land on the durable stream. The Session primitive preserves every observable effect regardless of agent success."* Then show the `session_update` storm in the events log.

**Hard recovery:** click **"Stop Runtime"** → click **"Launch Agent"** again. If the problem persists, fall back to a fresh **"Reset"** (which stops the runtime AND clears events) and try again. If still broken, see runbook §"Known issues — 5b".

### 5c. State explorer never populates

**Symptom:** after **"Launch Agent"** and **"New Session"**, the `sessions` tab on the right still shows *"Connecting durable state…"* forever, OR flashes *"State stream error: ..."* with a message about the stream URL.

**Cause most likely:** one of
- The vite proxy config got out of sync with the runtime port — it expects `127.0.0.1:4437` per `packages/browser-harness/vite.config.ts`, and the runtime bound somewhere else (or to a different interface).
- The runtime is embedding the durable-streams-server but hasn't published a `fireline-harness-state` stream yet because no session has been written to it (this is normal for the first ~100ms; if it persists beyond that, it's broken).
- The `@fireline/state` preload path in `createFirelineDB` errored — typically a schema mismatch.

**Say:** *"The state plane is decoupled from the ACP plane by design. Notice that the prompt flow still works — the Session primitive's *write* side is independent of the *read* side. The read side is a TanStack DB live query against a durable-streams subscription, and it's being served by the durable-streams-server embedded inside the `fireline` binary on port 4437. When the read side breaks, the substrate's core functionality is still intact; you've just lost the browser's materialized view."*

**Narrative recovery:** pivot to **"Here's what the inspector shows"** — the left pane's inspector card (**"Current Session"**) is not TanStack-backed; it reads from React state populated by the ACP and API responses. Walk through `status / sessionId / sessionStatus / lastError / handleId` and explain that these are the minimum fields any Host satisfier needs to expose. The state-explorer beat can be deferred to the fallback slide.

**Hard recovery:** in the browser console, check for network errors against `/v1/stream/fireline-harness-state`. If that path 404s, the runtime is healthy but hasn't created the stream yet (retry after 2–3 seconds). If it 502s, the vite proxy isn't reaching 4437 — see runbook §"Port and process table".

---

## Appendix — primitive-to-UI cross-reference

| UI element | Host verb | Primitive | TLA invariant (if any) |
|---|---|---|---|
| "Launch Agent" button | `host.provision(spec)` | Host + Sandbox | `ProvisionReturnsReachableRuntime` (line 814) |
| "New Session" button | ACP `session/new` via WebSocket (not a Host verb) | Session | `SessionAppendOnly` (line 755) |
| "Send" (prompt form) | ACP `session/prompt` | Harness + Session | `HarnessEveryEffectLogged` (line 776), `HarnessAppendOrderStable` (line 783) |
| "Reconnect + Load" button | ACP `session/load` | Session | `SessionDurableAcrossRuntimeDeath` (line 763) |
| "Disconnect" button | WebSocket close (not a Host verb) | Session (read side continues) | — |
| "Wake" button | `host.wake(handle)` | Orchestration | `WakeOnReadyIsNoop` (line 789), `ConcurrentWakeSingleWinner` (line 794), `WakeOnStoppedChangesRuntimeId` (line 802) |
| Resource discovery beat (§3) | `fireline publish-resource` + `DurableStreamMounter` | Resources | `ResourcePublishedIsEventuallyDiscoverable`, `SourceRefIsImmutableAfterPublish` (deployment_discovery.tla) |
| "Stop Runtime" button | `host.stop(handle)` | Host | (no direct invariant; complements the Wake pair) |
| "Reset" button | `disconnect + clear events` (not a Host verb) | — | — |
| State explorer tabs | `@fireline/state` live queries | Session (read surface) | `SessionAppendOnly` (line 755), `SessionScopedIdempotentAppend` (line 769) |
| Inspector card's `handleId` / `sessionStatus` | `host.status(handle)` polled after every verb | Host | — |

---

## TODO(demo-review) items captured inline

1. **§3 Wake moment** — the `WakeOnStoppedChangesRuntimeId` beat is a spec-level demonstration because the TS `createFirelineHost.wake` returns `blocked` on stopped instead of composing the cold-start recovery. Decide before demo day whether to (a) talk through it on a slide or (b) extend the TS satisfier.
2. **§1 Narrative** — the "crate layout inherits from primitive taxonomy" line is true as an in-progress claim (the Cargo workspace member registration in `3e06b86` is the scaffolding; the actual move into primitive-aligned crates is happening in `283a903`, `abd5a29`, and subsequent commits from workspace:13). If the restructure hasn't fully landed by demo day, soften to *"the restructure is in flight; you can see the target crates registered in the workspace already"* and point at `docs/proposals/crate-restructure-manifest.md`.
3. **§5a Fallback** — confirm the exact hand-start invocation of `fireline-control-plane` before demo. The dev-server.mjs invocation is the authoritative template (see `packages/browser-harness/dev-server.mjs:210-227`); copy it into the runbook verbatim.
4. **Agent selector** — the demo script assumes `fireline-testy-load` is the only launchable agent in the catalog. If any other agents have been registered by demo day, update the "Select **Fireline Testy Load (command)**" instruction in §2.1.
