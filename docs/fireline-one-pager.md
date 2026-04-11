# Fireline in One Page

**What:** A substrate for managed agents. Every surface Fireline exposes maps to one or two of Anthropic's six managed-agent primitives. Products like Flamecast build on top of Fireline without Fireline absorbing the product scope.

**Source post:** [*"Managed agents: a small set of primitives for any agent harness"*](https://www.anthropic.com/engineering/managed-agents), Anthropic engineering.

## Six primitives

| # | Primitive | Interface | Fireline status |
|---|---|---|---|
| 1 | **Session** | Append-only log, idempotent appends, replayable from any offset | **Strong** — `durable-streams` + `DurableStreamTracer` + runtime/TS materializers; `@fireline/state` collections as the TS read surface |
| 2 | **Orchestration** | `wake(session_id) → void` | **Live** — `Host.wake(handle)` trait + `whileLoopOrchestrator`; see `proposals/client-primitives.md` Modules 2–3 |
| 3 | **Harness** | `yield Effect → EffectResult` | **Partial by design** — conductor proxy chain; durable suspend/resume via the seven-combinator composition layer |
| 4 | **Sandbox** | `provision(resources) → execute(name, input)` | **Strong** — `Sandbox` trait in `fireline-conductor::primitives`; `MicrosandboxSandbox` satisfier behind `microsandbox-provider` feature |
| 5 | **Resources** | `[{source_ref, mount_path}]` | **Strong on ACP-fs and component-layer mounts** — `ResourceMounter` + `FsBackendComponent`; Docker-scoped physical-mount test remains as a cross-reference marker |
| 6 | **Tools** | `{name, description, input_schema}` | **Strong** — topology components + MCP injection (`PeerComponent`, `SmitheryComponent`, etc.); portable `CapabilityRef` / `TransportRef` / `CredentialRef` live as TS types in `@fireline/client/core` |

**Net status: no remaining primitive-sized gaps.** Every missing piece is a targeted small addition expressible as a combinator over the primitives above.

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

**Orchestration** looked like it needed a new scheduler service and a `wake(session_id)` HTTP endpoint. It doesn't. `durable-streams` accepts writes from any authenticated producer; any process that subscribes becomes a scheduler; `session/load` already rebuilds ACP session state from the durable log; the `Host` primitive's `wake(handle)` verb is retry-safe and idempotent. The TS surface ships this as `Host` + `Orchestrator` (see `proposals/client-primitives.md` Modules 2–3) plus a `whileLoopOrchestrator` satisfier and the `createFirelineHost` / `createClaudeHost` concrete hosts. No scheduler service. No new HTTP endpoint. No new primitive.

**Resources** splits cleanly. ACP defines `fs/read_text_file` and `fs/write_text_file` as client-hosted protocol methods — they flow through the conductor proxy chain like any other effect. An `FsBackendComponent` implemented as `compose(substitute, appendToSession)` routes file ops to any backend (local disk, S3, GCS, git, or even the Session log itself, which gives a cross-runtime virtual filesystem for free). Shell-based agents (Claude Code, Codex, etc.) bypass ACP fs and use the `ResourceMounter` trait for physical mounts at provision time — landed in `fireline-conductor::runtime::mounter`.

## Where to read more

- **`docs/explorations/managed-agents-mapping.md`** — the operational source of truth that drives the substrate roadmap. Start here for deep context and the seven-combinator algebra.
- **`docs/proposals/client-primitives.md`** — the authoritative TypeScript substrate surface: `Host`, `Sandbox`, `Orchestrator`, `Combinator`, and the module layout. This supersedes earlier TS-API exploration docs.
- **`docs/proposals/runtime-host-split.md`** §7 — the Host / Sandbox / Orchestrator reframe on the Rust side, and how the proposed internal split reconciles with the primitive taxonomy.
- **`docs/proposals/crate-restructure-manifest.md`** — the target Rust crate layout with 1:1 primitive alignment (`fireline-session`, `fireline-orchestration`, `fireline-harness`, `fireline-sandbox`, `fireline-resources`, `fireline-tools`).
- **`docs/explorations/managed-agents-citations.md`** — file:line inventory of which Rust code implements each primitive today.
- **`docs/explorations/claude-agent-sdk-v2-findings.md`** — verification of the Claude Agent SDK v2 preview against the `Host` primitive, used by the `createClaudeHost` satisfier.

If you're picking up new work, start from `managed-agents-mapping.md`. If you're writing a new component, express it as a combinator over the primitives first; if that doesn't work, either you've found a rare new primitive or the feature belongs in a downstream product layer.
