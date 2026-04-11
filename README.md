# Fireline

> Open-source substrate for managed agents. Every surface Fireline exposes maps to one of Anthropic's six managed-agent primitives. Products (Flamecast and others) build on top without Fireline absorbing their scope.

Fireline is the thing that makes an ACP agent **durable, observable, peerable, and resumable**. It runs the conductor that sits between an ACP client and an agent process, projects every effect into a durable `STATE-PROTOCOL` stream, and exposes a small set of trait-shaped primitives that downstream TypeScript consumers compose against.

Pairs with **[Flamecast](https://github.com/flamecast)** — the operator-facing control plane for agents that run on Fireline. Fireline is the substrate; Flamecast is the product layer above it.

## The six primitives

| # | Primitive | Interface | Fireline status |
|---|---|---|---|
| 1 | **Session** | Append-only log, idempotent appends, replayable from any offset | **Strong** — `durable-streams` + `DurableStreamTracer` + `@fireline/state` live collections |
| 2 | **Orchestration** | `wake(session_id) → void` | **Live** — `Host.wake(handle)` trait + `whileLoopOrchestrator` satisfier |
| 3 | **Harness** | `yield Effect → EffectResult` | **Partial by design** — conductor proxy chain; durable suspend/resume via seven-combinator composition |
| 4 | **Sandbox** | `provision(resources) → execute(name, input)` | **Strong** — `Sandbox` trait + `MicrosandboxSandbox` satisfier (behind `microsandbox-provider` feature) |
| 5 | **Resources** | `[{source_ref, mount_path}]` | **Strong** — `ResourceMounter` + `FsBackendComponent`; ACP-fs and physical-mount paths both live |
| 6 | **Tools** | `{name, description, input_schema}` | **Strong** — topology components + MCP injection; portable `CapabilityRef` / `TransportRef` / `CredentialRef` in `@fireline/client/core` |

Fireline introduces concepts **above** the six primitives (conductor components, topology specs, proxy chains, materializers) but **none of these are new primitives** — they all decompose into a seven-combinator algebra (`observe`, `mapEffect`, `appendToSession`, `filter`, `substitute`, `suspend`, `fanout`) over the primitive substrate. See `docs/explorations/managed-agents-mapping.md` for the full decomposition.

## Authoritative references

Start here, in this order:

- **`docs/proposals/client-primitives.md`** — the authoritative TypeScript substrate surface (`Host`, `Sandbox`, `Orchestrator`, `Combinator`, module layout). This supersedes every earlier TS-API exploration doc.
- **`docs/proposals/runtime-host-split.md`** §7 — Host / Sandbox / Orchestrator reframe on the Rust side.
- **`docs/proposals/crate-restructure-manifest.md`** — target Rust crate layout with 1:1 primitive alignment.
- **`docs/explorations/managed-agents-mapping.md`** — operational source of truth for the six primitives and the combinator algebra.
- **`docs/explorations/managed-agents-citations.md`** — file:line inventory of where each primitive is implemented today.
- **`docs/explorations/claude-agent-sdk-v2-findings.md`** — verification of the Claude Agent SDK v2 preview against the `Host` primitive.
- **`docs/architecture.md`** — comprehensive architectural reference.

## Repo layout (target)

The target Rust workspace layout below is aligned 1:1 with the primitive taxonomy per `docs/proposals/crate-restructure-manifest.md`.

> **Note:** **target layout — restructure in progress.** The live tree still contains `crates/fireline-conductor` and `crates/fireline-components` as transitional crates being dissolved into the primitive-aligned crates below. Use the manifest for the current move state.

```
fireline/
├── Cargo.toml                       # workspace root + binary package
├── crates/
│   ├── fireline-semantics/          # pure semantic kernel (leaf, no internal deps)
│   ├── fireline-session/            # durable-stream-backed session log + replay
│   ├── fireline-orchestration/      # wake(session_id), trigger loop, session index
│   ├── fireline-harness/            # ACP adapter, approval gate, effect capture
│   ├── fireline-sandbox/            # tool execution container (microsandbox, local, docker)
│   ├── fireline-resources/          # mount + fs backend + resource attachment
│   ├── fireline-tools/              # registry, capability ref, descriptor projection
│   ├── fireline-runtime/            # runtime manager + provider + registry glue
│   └── fireline-control-plane/      # HTTP runtime management surface
├── packages/                        # TypeScript workspace (pnpm)
│   ├── state/                       # @fireline/state — durable-stream consumer collections
│   ├── client/                      # @fireline/client — Host / Sandbox / Orchestrator surface
│   │   └── src/
│   │       ├── core/                # @fireline/client/core — combinators, refs, specs
│   │       ├── host/                # @fireline/client/host — Host primitive + types
│   │       ├── orchestration/       # @fireline/client/orchestration — whileLoopOrchestrator
│   │       ├── host-fireline/       # createFirelineHost satisfier
│   │       └── host-claude/         # createClaudeHost satisfier (Agent SDK v2 preview)
│   └── browser-harness/             # Vite dev harness driven by the Host primitive
├── src/                             # fireline binary
│   ├── main.rs                      # CLI + bootstrap
│   └── bin/                         # additional binaries (dashboard, agents CLI, testy variants)
├── tests/                           # cross-crate Rust integration tests
├── verification/                    # Stateright + TLA model-checking for the semantic kernel
└── docs/                            # architecture and design docs
    ├── architecture.md
    ├── proposals/                   # authoritative proposals (client-primitives, host-split, restructure)
    └── explorations/                # source-of-truth reference docs
```

## Status

- **Managed-agent suite is green on CI.** 30+ managed-agent-suite tests plus `runtime_index_agreement` pass in the default feature configuration.
- **Stream-as-truth refactor.** Phase 1 is landed (`runtime_endpoints` envelope + `RuntimeIndex` projection + agreement tests). Phase 2/3 (flip read path, delete `RuntimeRegistry`) are deferred pending production control-plane shared-state subscription.
- **Primitive trait layer is live.** `Host`, `Sandbox`, `Orchestrator` traits introduced in `crates/fireline-conductor/src/primitives/` (pending migration into `fireline-runtime` per the restructure manifest).
- **`MicrosandboxSandbox` satisfier** ships behind the `microsandbox-provider` feature flag (off by default — libdbus-sys doesn't build on vanilla CI runners).
- **TypeScript primitive layer** (Tiers 1–5) is landed: `@fireline/client/core`, `/host`, `/orchestration`, `/host-fireline`, `/host-claude`, and the browser-harness rewire onto the `Host` primitive.

For an operational snapshot and the live in-flight work index, see `docs/handoff-2026-04-11-stream-as-truth-and-runtime-abstraction.md`.

## License

TBD. See `LICENSE` (to be added).
