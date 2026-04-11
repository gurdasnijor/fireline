# Fireline in One Page

**What:** A substrate for managed agents. Every surface Fireline exposes maps to one or two of Anthropic's six managed-agent primitives. Products like Flamecast build on top of Fireline without Fireline absorbing the product scope.

**Source post:** [*"Managed agents: a small set of primitives for any agent harness"*](https://www.anthropic.com/engineering/managed-agents), Anthropic engineering.

## Six primitives

| # | Primitive | Interface | Fireline status |
|---|---|---|---|
| 1 | **Session** | Append-only log, idempotent appends, replayable from any offset | **Strong** — `durable-streams` + `DurableStreamTracer` + runtime/TS materializers |
| 2 | **Orchestration** | `wake(session_id) → void` | **Composable** — satisfied by existing primitives, no new surface |
| 3 | **Harness** | `yield Effect → EffectResult` | **Partial by design** — conductor proxy chain; durable suspend/resume via composition |
| 4 | **Sandbox** | `provision(resources) → execute(name, input)` | **Strong** — `RuntimeProvider` trait, local + Docker providers |
| 5 | **Resources** | `[{source_ref, mount_path}]` | **Partially composable** — ACP fs interception is pure composition; physical mounts for shell-based agents need small slice 15 work |
| 6 | **Tools** | `{name, description, input_schema}` | **Strong** — topology components + MCP injection (`PeerComponent`, `SmitheryComponent`, etc.) |

**Net status: no remaining primitive-sized gaps.** Every missing piece is a targeted small addition that folds into an existing slice.

## What Fireline owns vs doesn't

**Owns** — runtime lifecycle and discovery, durable session evidence, canonical read surfaces, the conductor proxy chain for composing behavior around agents, pause/wait/resume mechanics, external credential reference seams.

**Doesn't own** — human-facing control plane products, agent identity, wallets or payment rails, marketplace and service catalog UX, multi-tenant product databases for user-facing objects. Those belong to products built *on top of* Fireline.

## Composition, not new abstractions

Above the six primitives, Fireline's conductor components, topology specs, proxy chains, and materializers are all **functional composition** — no new primitives. Every conductor component is a `Harness → Harness` transformer, and seven base combinators (`observe`, `mapEffect`, `appendToSession`, `filter`, `substitute`, `suspend`, `fanout`) cover every component we have today.

Building a topology is just `compose(...)`:

```typescript
const topology = compose(
  audit(),                                          // = appendToSession
  contextInjection({ sources }),                    // = mapEffect
  approvalGate({ scope: 'tool_calls' }),            // = suspend
  budget({ tokens: 1_000_000 }),                    // = filter
  peer({ peers: ['runtime:reviewer'] }),            // = substitute
)
```

Tools are Components (`mapEffect` over init). Resources are Components (init-time mount). Materializers are a second fold family (`(Event, S) → S`) over the Session log. The whole substrate is compositions, not abstractions.

If a proposed feature doesn't decompose into the combinator algebra, it's either a rare new primitive (only Orchestration and Resources ever qualified, and both turned out to be composable) or a product-layer object that belongs in Flamecast.

## Two "primitive-sized gaps" that collapsed into composition

**Orchestration** looked like it needed a new scheduler service and a `wake(session_id)` HTTP endpoint. It doesn't. `durable-streams` accepts writes from any authenticated producer; any process that subscribes becomes a scheduler; `session/load` already rebuilds ACP session state from the durable log; `RuntimeHost::create` cold-starts runtimes. Composed via a ten-line `resume(sessionId)` helper:

```typescript
async function resume(sessionId: string) {
  const session = await sessionStore.get(sessionId)
  let runtime = await getRuntime(session.runtimeKey)
  if (!runtime || runtime.status !== 'ready') {
    runtime = await provision(session.runtimeSpec)  // cold-start
  }
  const acp = await connectAcp(runtime.acp)
  await acp.loadSession(sessionId)                   // rebuild from log
}
```

No scheduler service. No new HTTP endpoint. No new primitive.

**Resources** splits cleanly. ACP defines `fs/read_text_file` and `fs/write_text_file` as client-hosted protocol methods — they flow through the conductor proxy chain like any other effect. An `FsBackendComponent` implemented as `compose(substitute, appendToSession)` routes file ops to any backend (local disk, S3, GCS, git, or even the Session log itself, which gives a cross-runtime virtual filesystem for free). Shell-based agents (Claude Code, Codex, etc.) bypass ACP fs and still need physical mounts at provision time — ~1 week of Rust work in slice 15 for the `ResourceMounter` trait.

## What's actually left to build

| # | Slice | Scope | Status |
|---|---|---|---|
| 13c | Docker provider via `bollard` | Sandbox depth | In flight |
| 14 | Session canonical read schema + durable `runtimeSpec` persistence | Session + Orchestration composition enabler | Doc rewritten, implementation pending |
| 15 | `ResourceMounter` + `FsBackendComponent` | Resources | Doc rewritten, implementation pending (~1.5 weeks) |
| 16 | `ApprovalGateComponent` rebuild-from-log + `resume` worked example | First Orchestration composition consumer | Doc rewrite in progress |
| 17 | Portable Tools references with `credential_ref` | Tools depth | Doc rewrite pending |
| TS API | `resume` helper, `fsBackend` factory, materialize helpers | Client-side composition | Proposal in `typescript-functional-api-proposal.md` |
| Ongoing | Richer budget, routing, delegation components | Component depth | Additive on existing topology |

**Zero new primitives. Zero new control-plane endpoints. No slice 18.** Every remaining item is either composition of what exists or a targeted small extension.

## Full detail

- **`docs/explorations/managed-agents-mapping.md`** — the 801-line operational source of truth that drives the slice plan. Start here for deep context.
- **`docs/explorations/typescript-functional-api-proposal.md`** — proposed TypeScript API shape grounded in the seven-combinator algebra.
- **`docs/explorations/managed-agents-citations.md`** — file:line inventory of which Rust code implements each primitive today.
- **`docs/product/priorities.md`** — substrate-first product positioning and slice ordering.
- **`docs/runtime/control-and-data-plane.md`** — the two-plane architecture the primitives map onto.

If you're picking up new work, start from `managed-agents-mapping.md`. If you're writing a new component or slice, express it as a combinator over the primitives first; if that doesn't work, either you've found a rare new primitive or the feature belongs in Flamecast.
