# Programmable Topology — Exploring the Cross-Cutting Concerns Agent Users Actually Want

> Status: Exploration / design-space inventory
> Type: Forward-looking, not a committed spec
> Scope: what kinds of components belong in the `client.topology` layer, which user pains justify them, which are highest-leverage, and how they extend the TS primitive API surface.
> Related:
> - [`ts/primitives.md`](./ts/primitives.md)
> - [`research/adjacent-systems.md`](./research/adjacent-systems.md)
> - Fireline `docs/state/runtime-materializer.md`
> - SDK `ConnectTo` trait (agent-client-protocol-core/src/component.rs)

## Purpose

Fireline today uses the SDK's `ConnectTo` trait for exactly one thing — `PeerComponent`, which injects an MCP server that exposes cross-agent calls. The trait itself is **chainable middleware for ACP traffic**: every proxy in the chain sees every message, can transform it, inject new messages, generate its own replies, or cause side effects. One component exercises maybe 10% of that design space.

This document inventories the rest. Specifically:

1. What cross-cutting concerns do agent-harness users actually need to layer on that they can't do today without forking the harness?
2. Which of those concerns are highest-ROI to build first?
3. How do those components project into the TS primitive API surface (`client.topology`, `client.state`, …) defined in [`ts/primitives.md`](./ts/primitives.md)?
4. What's missing from that primitive surface today to support programmable topology?

This is not a committed spec. It is a design-space survey to inform which components to build, in what order, and how the TS layer should grow to describe them.

## The cross-cutting concerns agent users actually want

These are the real pain points people hit when they try to build serious agent workflows on top of an ACP/MCP harness. Each one, today, requires either (a) forking the harness, (b) reimplementing part of the agent protocol, or (c) accepting you can't have it. Organized by user pain, not by component shape.

### 1. Safety and control

| Concern | What users want | Impossible without forking today? |
|---|---|---|
| Budget caps | "Stop this session after 50k tokens / 10 minutes / 100 tool calls." | Yes |
| Approval gates | "Ask me before the agent runs any shell command." | Partially — ACP has a permission primitive but no composable policy layer |
| Tool allow/deny lists | "This session can only use `read_file` and `grep`." | Yes |
| Per-tool quotas | "Max 20 `shell` calls per session." | Yes |
| Scope restriction | "Agent can only touch files under `/workspace`." | Yes |
| Audit trail | "Log every message and tool call to a compliance sink." | Partially — the state stream captures some of this but not in a compliance shape |

### 2. Context and memory

| Concern | What users want | Impossible today? |
|---|---|---|
| Persistent memory | "Remember what we talked about last week." | Yes |
| Workspace instructions | "Always follow this project's style guide." | Partially — `CLAUDE.md`-style works but isn't composable |
| Session continuity | "The new session should know what the prior session accomplished." | Yes |
| Runtime state injection | "Agent should know it currently has 3 unmerged PRs." | Yes |
| Operator messages | "When I type `/status`, inject the CI build status into the next prompt." | Yes |
| Persona / personality | "Always start conversations in this tone." | Partially — prompt engineering handles most of this |

### 3. Observability and debugging

| Concern | What users want | Impossible today? |
|---|---|---|
| Full-fidelity session recording | "Record every byte for later replay." | Partially — the state stream is normalized, not raw |
| Deterministic replay | "Reproduce this bug against a canned session." | Yes |
| Metrics | "Latency per turn, tokens per session, error rates." | Yes |
| OpenTelemetry export | "Ship traces to our existing collector." | Yes |
| Diff between runs | "How did agent B answer this prompt vs agent A?" | Yes |
| LLM-as-judge evaluation | "Score each turn for correctness / safety / helpfulness." | Yes |

### 4. Multi-agent orchestration

| Concern | What users want | Impossible today? |
|---|---|---|
| Best-agent routing | "Send DB questions to the SQL expert, code questions to the code expert." | Yes — Fireline has peer calls but no routing layer |
| Fan-out / consensus | "Ask 3 agents in parallel, return the majority answer." | Yes |
| Handoff | "Agent A starts this task, agent B takes over when it needs security review." | Yes |
| Bounded sub-agents | "Spawn a sub-agent with only read access, can't write files." | Yes |
| Team memory | "All agents share the same working memory." | Yes |

### 5. UX and product features

| Concern | What users want | Impossible today? |
|---|---|---|
| Branching / tangents | "Explore this side question, then resume the main conversation." | Yes — Niko's tangent proxy example |
| Time-travel | "Go back to turn 5 and try a different approach." | Yes |
| Session sharing | "Let someone else watch this session in real time." | Partially — requires consumer changes |
| Interventions | "Inject 'be more careful' without disrupting the agent's flow." | Yes |
| Walkthrough rendering | "Convert this walkthrough markdown into a sidebar." | Yes — Niko's walkthrough proxy example |

### 6. Integration and extensibility

| Concern | What users want | Impossible today? |
|---|---|---|
| Webhook on state transitions | "POST to Slack when a permission request happens." | Partially — a shim exists in durable-acp-rs |
| Auto-commit at session end | "Commit whatever the agent wrote with a generated message." | Yes |
| Snapshot before/after | "Take a git snapshot before and after each turn." | Yes |
| DB transaction wrapping | "Wrap the whole session in a DB transaction; rollback on error." | Yes |

### 7. Testing and evaluation

| Concern | What users want | Impossible today? |
|---|---|---|
| Record-replay fixtures | "Use recorded sessions as deterministic test cases." | Yes |
| Prompt injection detection | "Flag prompts that look like attempted jailbreaks." | Yes |
| Output assertions | "Fail the test if the agent suggests `rm -rf`." | Yes |
| Canary / A-B | "Run the new agent version on 5% of sessions." | Yes |
| Load testing | "Simulate 100 concurrent sessions." | Partially |

## Mapping to component shapes

Grouping the concerns above by *how* they interact with the protocol — this is the shape of the `ConnectTo` implementation each one would take.

**Inbound transformers** (modify `session/prompt` before it reaches the agent)
- Context injection, persona, persistent memory, operator messages, workspace instructions
- Prompt injection detection, input guardrails, prompt rewriting

**Outbound transformers** (modify agent notifications before they reach the client)
- Redaction, PII scrubbing
- Walkthrough rendering, rich-content augmentation
- Summary condensation

**Pause-and-escalate** (hold a message, ask the user / a sidecar, resume)
- Approval gates
- Human intervention during errors

**Pure observers** (see everything, affect nothing on the data plane)
- Audit log, metrics, OpenTelemetry export
- Full-fidelity recording for replay
- Diff-against-recording, assertion checking

**Stateful gates** (maintain state across messages in a session)
- Budget caps (token counter)
- Rate limiters (tool call counter)
- Circuit breakers (consecutive error counter)

**Chain terminators** (act as the agent, don't forward)
- Replay proxy (plays back a recorded session)
- Mock agent (canned responses for tests)
- Ensemble / consensus aggregator (fans out, waits, aggregates)

**MCP tool injectors** (expose new MCP tools to the agent)
- Peer calls (Fireline already has this)
- Memory save/recall
- Knowledge base search
- Operator intervention primitives
- Any custom tool

**Cross-chain coordinators** (affect other proxies in the chain)
- Shared session state, shared audit sinks, shared budget pools

## ROI ranking — what to build first

Ranking by `(user demand × current painfulness) / implementation cost`, biased toward "concrete user need, small build, large leverage."

### Tier 1 — ship these first, highest leverage per line of code

1. **Audit proxy** (pure observer, writes to a separate stream). Every serious user needs a compliance trail. Nobody offers it cleanly today. Implementation is small — the state stream infrastructure already exists, this adds a second stream with different retention. Bonus: demonstrates that Fireline can host multiple durable streams for multiple consumers.

2. **Approval gate proxy** (pause + escalate via `session/request_permission`). Every user who lets an agent touch production needs this. The SDK permission primitive exists but isn't exposed as a composable policy — this makes it one. Composes immediately with `PeerComponent` (approve peer calls), shell tools, writes, and any pattern-matched tool call.

3. **Context injection proxy** (inbound transformer reading from the durable stream). Everyone wants persistent memory, workspace instructions, or recent-session context. This is the "the durable stream is useful to agents in real time, not just to observers" demonstration. Composes with itself — stack multiple sources for a layered context budget.

4. **Budget gate proxy** (stateful counter, terminates when exceeded). Every user running agents at scale needs this. Zero external dependencies.

### Tier 2 — high value, medium build

5. **Recording + replay pair.** `RecordingProxy` captures raw envelopes to a dedicated stream; `ReplayProxy` acts as an agent playing back a recording. Unlocks deterministic testing, bug reproduction, regression fixtures, client integration tests. The durable substrate pays for itself here — a VCR is the canonical feature.

6. **Redaction proxy** (outbound pattern match → `[REDACTED]`). Compliance-driven, always runs last in the outbound direction. Small, composable with anything.

7. **Metrics proxy** (pure observer, exposes `/metrics`). Ops concern, users already have Prometheus-shaped tooling. Very small build.

8. **Router / fan-out proxies** (extend the peer component with selection + broadcast). Next evolution of the existing peer story; turns Fireline from 1:1 peering into small-team orchestration.

### Tier 3 — valuable but niche or larger

9. **OpenTelemetry export proxy.** For teams with existing tracing infra. Real need but narrow audience.
10. **Tangent / branching proxy** (Niko's example). Cool UX, specific enough that it probably deserves its own design pass.
11. **LLM-as-judge evaluation proxy.** Research tool, production value unclear.
12. **Prompt injection detection proxy.** Real security concern, but unclear whether a proxy is the right home (vs. a guardrail library inside the agent).

### Tier 4 — defer until concrete pull

Everything else: diff, fuzz, snapshot-before-after, transaction wrapping, canary routing, walkthrough rendering. Valuable in specific contexts, too speculative to build without a real consumer asking.

## What forces users to fork the harness today

A sharper framing of ROI: **which concerns specifically force users to fork the harness today?** That's the load-bearing question — if an external tool can already serve the need, the proxy's leverage is smaller.

| Concern | Fork-forcing? |
|---|---|
| **Budget caps** | ✅ No harness exposes composable budget limits |
| **Approval gates with custom policy** | ✅ Permission primitive exists, policy layer does not |
| **Audit to a specific compliance sink** | ✅ Users tap internal state manually |
| **Context injection from a specific backend** | ✅ Users hack agent startup scripts |
| **Full-fidelity recording** | ✅ State stream is normalized, not byte-level |
| **Multi-agent orchestration beyond 1:1** | ✅ Users build their own control plane or fork |
| Metrics | ❌ Langfuse, Helicone, Grafana Agent partially serve this |
| Persona / personality | ❌ System-prompt engineering handles most of it |
| Webhook delivery | ❌ External webhook services can consume a single event stream |

The checked items are the ROI-concentrated picks. **Audit, approval gates, budget caps, context injection, full-fidelity recording, multi-agent orchestration.** These are exactly the six concerns worth building first-class component support for — they're the things that today's users are already paying a "fork the harness" cost to solve, and each one reduces to a component that fits the chain naturally.

## Projecting through the TS primitive API

The current [`ts/primitives.md`](./ts/primitives.md) doc now reserves the
right namespace for this work: **`client.topology`**.

That namespace is still only documented, not implemented. It is the correct
seam for programmable runtime composition and should stay there rather than
being folded into `client.host` or `client.acp`.

The intended builder shape is:

```ts
const topology = client.topology
  .builder()
  .proxy("peer_mcp")
  .terminal("codex-acp", { kind: "subprocess" })
  .build();
```

This is the correct seam. The only work is to:

1. **Grow the component vocabulary.** Each proxy above gets a registered name + config schema.
2. **Teach the runtime to resolve names.** Rust maintains a `ComponentRegistry` keyed by name; each entry is a factory that takes a config and produces a `DynConnectTo<Conductor>`.
3. **Teach the TS builder about the catalog.** Typed builder methods for each registered component, so clients get compile-time validation.

Concretely, the builder grows to cover the tier-1 and tier-2 components:

```ts
const topology = client.topology
  .builder()

  // tier 1
  .audit({
    sinkStreamUrl: "http://.../streams/audit-agent-a",
    retention: "90d",
  })
  .approvalGate({
    policies: [
      { match: { tool: "shell" }, action: "require-approval" },
      { match: { tool: "prompt_peer" }, action: "require-approval" },
      { match: { tool: "write_file", pathPrefix: "/etc" }, action: "deny" },
    ],
  })
  .contextInjection({
    sources: [
      { kind: "memory-stream", streamUrl: "http://.../streams/memory-agent-a" },
      { kind: "workspace-file", path: "CLAUDE.md" },
      { kind: "datetime" },
    ],
  })
  .budget({
    maxTokens: 50_000,
    maxToolCalls: 200,
    maxDurationMs: 600_000,
    onExceeded: "terminate-turn",
  })

  // existing
  .peerMcp()

  // tier 2
  .recording({ sinkStreamUrl: "http://.../streams/recording-agent-a" })
  .redaction({ patterns: [/sk-[a-z0-9]+/i, /password=\S+/i] })
  .metrics({ exposeAt: "/metrics" })

  .terminal("claude-acp", { kind: "subprocess" })
  .build();
```

Then either at runtime creation:

```ts
const runtime = await client.host.create({
  provider: "auto",
  agent: "claude-acp",
  topology,
});
```

…or at initialize-time via the metadata channel the TS doc already names:

```ts
const acp = await client.acp.connect({ url: runtime.acpUrl });
await acp.initialize({
  meta: { "durable-acp/topology": topology },
});
```

Both already work under the existing primitive model. **Nothing in the TS surface needs to be reshaped — only extended.**

## What's missing from the current TS primitive surface

Supporting programmable topology exposes five gaps in
[`ts/primitives.md`](./ts/primitives.md) worth filling.

**1. Component discovery.** Clients need `client.topology.listComponents()` to see what's registered on a given runtime — otherwise `.audit(...)` compiles fine but fails at runtime if the server doesn't know the name. Needed for dynamic or config-driven topology, and for any kind of "which harness am I talking to?" introspection.

**2. Per-component config schemas.** Each registered component should publish a JSON schema. `client.topology.describeComponent("audit")` returns the schema. TS bindings can be generated from it. This is how `@fireline/client` stays in sync with the Rust `ComponentRegistry` without a hand-maintained catalog drifting across language boundaries.

**3. Ordering constraints.** Some components have ordering rules. `RedactionProxy` must run last in the outbound direction. `AuditProxy` should see raw traffic (so it must be inbound-first and outbound-last, i.e. wrap the whole chain). `ApprovalGateProxy` must see a message before `PeerComponent` does. The topology spec needs a way to express these constraints, and the runtime needs to validate at build time. Two approaches:
- **Explicit**: `{ order: "last", after: ["audit"] }` per component. Flexible but verbose.
- **Phase-based**: assign each component a phase (`observe | transform-inbound | gate | forward | transform-outbound`); the builder auto-sorts within phase. Simpler, less flexible.
The phase approach is likely sufficient for v1.

**4. Runtime-scoped services.** Proxies like `ContextInjectionProxy` need access to *other* runtime-provided resources: the durable stream for reads, the state materializer, the producer for writes, an HTTP client pool. They shouldn't reach out to their own infrastructure. The runtime needs a **service locator** components request things from at construction time. In Rust this is probably a `ComponentContext { producer, materializer, state_stream_url, config, .. }` passed to the factory. In TS it isn't visible — it's a server-side concern — but the design has to exist before component factories can be written.

**5. Shared streams across components.** `AuditProxy` writes to one stream, `RecordingProxy` to another, `ContextInjectionProxy` reads from a third. The runtime should manage streams centrally so proxies don't duplicate subscriber loops or fight over producer handles. This extends the existing runtime materializer pattern: streams are declared in the runtime config, proxies request them by name through the service locator.

Gap 5 connects directly to the runtime materializer pattern Fireline just built (see [`fireline/docs/state/runtime-materializer.md`](../../../fireline/docs/state/runtime-materializer.md)). The materializer already registers projections against one stream. The extended version lets proxies register against *multiple* streams, with the runtime managing subscriber loops and producer handles centrally.

## Composition matrix — which components compose

A sanity check: do these actually compose without weird interactions?

| First ↓ Second → | Audit | Approval | Context | Budget | Peer | Redact | Record |
|---|:-:|:-:|:-:|:-:|:-:|:-:|:-:|
| **Audit** | — | ✓ | ✓ | ✓ | ✓ | ⚠ sees pre-redact | ✓ separate streams |
| **Approval** | ✓ | — | ✓ | ✓ | ✓ gates peer | ✓ | ✓ |
| **Context** | ✓ | ✓ | ✓ stack! | ✓ | ✓ | ✓ | ✓ |
| **Budget** | ✓ | ✓ | ✓ | — | ✓ peer counts | ✓ | ✓ |
| **Peer** | ✓ | ✓ gated | ✓ | ✓ | — | ✓ | ✓ |
| **Redact** | ⚠ should see raw | ✓ | ✓ | ✓ | ✓ | — | ⚠ should record raw |
| **Record** | ✓ | ✓ | ✓ | ✓ | ✓ | ⚠ | — |

Two yellow flags worth naming now:

- **Redact vs Audit/Record ordering.** Audit and Record want to see *pre-redacted* messages (full fidelity); the wire-level downstream consumer wants *post-redacted* output. Audit and Record must run BEFORE Redact in the outbound direction. This is exactly the ordering-constraint gap — `phase: "observe"` runs before `phase: "transform-outbound"` gets it right automatically.

- **Budget counting peer calls.** If peer calls are MCP tool calls, the budget proxy sees them and counts them. If peer calls happen "outside" the MCP boundary (fresh ACP connections), the budget may or may not see them depending on where it sits in the chain. Decision needed: does the budget proxy count peer calls as tool calls, as sub-session work, or separately? Probably "as tool calls" for v1 (simpler) with an opt-in for "count descendant mesh turns" later.

These are good questions to answer before building, not blockers.

## A first-mover slice

The smallest slice that demonstrates the whole pattern end-to-end is **three components**: `Audit`, `ContextInjection`, and the existing `Peer`. Build order:

1. **`fireline-audit`** crate (~200 lines). Implements `AuditProxy` as `impl ConnectTo<Conductor>`. Pure observer. Registers with `ComponentRegistry` as `"audit"`. Writes to a configurable stream URL via a producer obtained from `ComponentContext`.

2. **`fireline-context`** crate (~250 lines). Implements `ContextInjectionProxy`. Inbound transformer on `session/prompt`. Reads from configurable sources (memory-stream, workspace-file, datetime for v1). Composes into multi-source stacking for v1.1.

3. **`ComponentRegistry` + `ComponentContext`** in `fireline-conductor` (~150 lines). Factories keyed by name; context carries the producer, the runtime materializer handle, the state stream URL, and a small HTTP client pool. Wire `PeerComponent` through the same registry (rename registration to `"peer_mcp"`).

4. **`bootstrap.rs` accepts a `TopologySpec`.** Start with runtime-creation-time
   config only. Build the chain from the registry rather than the current
   hard-coded `vec![LoadCoordinator, Peer]`.

5. **TS `@fireline/client` extension.** Grow `client.topology.builder()` with
   typed methods: `.audit({...})`, `.contextInjection({...})`, `.peerMcp()`,
   `.terminal(...)`. Generate from a manually-maintained catalog for now;
   automate from a server-published schema later (gap 2).

6. **End-to-end test.** A client builds a topology with all three components, passes it via `client.host.create({ topology })` or `acp.initialize({ meta: { "durable-acp/topology": topology }})`, and the runtime materializes the chain. Verify in both streams that audit records are being written and that context is being injected into prompts before the agent sees them.

That is one end-to-end vertical through the whole programmable-topology story. If it works, every additional proxy is incremental: each new concern is roughly 150-300 lines of Rust + a builder method in TS + a catalog entry + a JSON schema for config.

## What this reveals about the primitive API

Net findings against [`ts/primitives.md`](./ts/primitives.md):

- **`client.topology` is correctly shaped as the home** — no restructuring needed.
- **The `TopologySpec` JSON shape in the doc is sufficient** for the v1 slice (kind/name/config is enough for three components).
- **The doc is silent on component discovery** — worth adding as Q6 in Open Questions.
- **The doc is silent on per-component config schemas** — worth adding as Q7.
- **The doc is silent on ordering / phase constraints** — worth adding as Q8.
- **The doc's `client.topology.fromInitialize(ctx => ...)` example** (dynamic topology from initialize metadata) actually *implies* component discovery — you can't dynamically pick components you don't know about. Worth making that dependency explicit.
- **Runtime-scoped services (gap 4)** are a Rust-side concern not visible to the TS API, but the doc's "Mapping to Rust Backing Surfaces" table should acknowledge them alongside the other runtime internals.

Suggested follow-up PR to [`ts/primitives.md`](./ts/primitives.md):
1. Add component discovery + config schemas as sub-topics under `client.topology`.
2. Mention ordering / phase constraints in the topology section.
3. Add Q6 (discovery), Q7 (schemas), Q8 (ordering) to Open Questions.

None of that blocks the first-mover slice — it's about keeping the north-star doc honest as the component layer grows.

## Open questions

**Q1: Component registry — where does it live?** Most likely `fireline-conductor` defines a `ComponentRegistry` trait and factories; individual components live in their own small crates (`fireline-audit`, `fireline-context`, existing `fireline-peer`). The binary's `bootstrap.rs` wires them together. Alternatively the registry could live in a dedicated `fireline-topology` crate if the component count grows.

**Q2: Typed or dynamic config?** TS passes JSON, Rust deserializes with serde. Each component defines its own config struct. JSON schema is generated from the struct for discovery. This matches how everything else in the repo already works.

**Q3: When is topology decided — at runtime creation or at session initialization?** The TS design doc allows both. Creation-time is simpler and matches the current hard-coded chain. Init-time is more flexible but implies the runtime can swap chains per-session, which is a bigger ask. **Recommendation: start creation-time, add init-time later if a concrete use case demands it.**

**Q4: What happens if a client requests a component the runtime doesn't know?** Fail the `initialize` or `host.create` with a clear error listing the available components. Never silently drop — silent drops turn topology errors into mysterious behavior changes.

**Q5: Can proxies communicate directly?** E.g., can the budget proxy read from a shared counter used by the audit proxy? Initial answer: **no**, components are isolated. Shared state goes through a durable stream (write + read back via materializer). This preserves the producer-only architecture direction and means composition is always "via the substrate," not "via ambient state." Later, if performance demands, we can add a scoped shared-state service in `ComponentContext` — but not before it's needed.

**Q6: Do components need to participate in `session/load`?** Today `LoadCoordinatorComponent` handles this. For new components, what happens on reattach? Audit proxy: nothing, it's stateless. Budget proxy: needs to restore its counter from the audit or state stream. Context proxy: rereads its sources. This suggests a component-level "resume" hook in addition to a construction hook. **Defer until at least one component actually needs per-session restoration.**

**Q7: How are per-session component configs expressed?** A single topology spec applies to every session on a runtime by default. Some components may want per-session config overrides (different budget per session, different audit tag). The `acp.initialize({ meta: { "durable-acp/topology-overrides": ... }})` channel is the natural place, but the schema needs to express "what's overridable." Probably start by allowing the whole topology to be replaced via init metadata and adding partial overrides later.

## Next steps

1. Circulate this doc for review. The point is to agree on which tier-1 components to build and in what order, not to commit to every shape above.
2. Pick the first-mover slice (default recommendation: `Audit` + `ContextInjection` + existing `Peer`, projected through the TS builder with three new methods).
3. Write a narrower SDD for the chosen slice that names the `ComponentRegistry` shape, the `ComponentContext`, the exact Rust and TS extension points, and the wire format for `TopologySpec`.
4. Amend [`ts/primitives.md`](./ts/primitives.md) with the five gaps named in
   "What's missing from the current TS primitive surface."
5. Ship the slice. Each additional proxy after the first three is incremental, and the pattern proves itself through use rather than through more design docs.

The long-term pitch: **Fireline becomes the programmable topology runtime for ACP-speaking agents — the place where cross-cutting concerns live as first-class, composable components rather than as harness forks.** The durable stream substrate is what makes this honest (components can persist state, share via streams, and be independently replaceable), and `client.topology` is the surface where users describe their chain. Everything else is filling in the catalog.
