# Proposal: Option C Rust Combinator Serde Format

> **SUPERSEDED** by [`./client-api-redesign.md`](./client-api-redesign.md) and [`./sandbox-provider-model.md`](./sandbox-provider-model.md). Retained for architectural history.
> **Status:** design only
> **Type:** execution-driving proposal
> **Audience:** maintainers landing the post-restructure Rust-side combinator path
> **Source of truth for wire shape:** [`../../packages/client/src/core/combinator.ts`](../../packages/client/src/core/combinator.ts)
> **Related:**
> - [`./client-primitives.md`](./client-primitives.md) — authoritative TS substrate surface
> - [`../explorations/managed-agents-mapping.md`](../explorations/managed-agents-mapping.md) — the seven combinator semantics
> - [`./crate-restructure-manifest.md`](./crate-restructure-manifest.md) — post-restructure crate ownership

## Purpose

Option C is the Rust-side fix for the topology wire mismatch. The TypeScript client already models topology as `readonly Combinator[]`; the Rust runtime still expects legacy `{ components: [{ name, config }] }`. This proposal defines the **post-restructure** Rust serde shape and interpreter boundary that consume the TS combinator payload directly, without reintroducing a translator module on either side.

The target outcome is:

- `CreateRuntimeSpec.topology` carries a serialized `Vec<Combinator>`
- `fireline-harness` deserializes that payload exactly as TS emitted it
- a `CombinatorInterpreter` reduces each combinator into the harness effect pipeline in order
- the legacy named-component wire contract stops being part of the public launch API

## 1. Target Serde Shape

### 1.1 Top-level wire contract

The top-level topology wire value is the TS shape directly:

```text
topology: [Combinator, Combinator, ...]
```

Not:

```text
topology: { components: [...] }
```

An empty topology is `[]`. Once Option C lands, the legacy `{ components: [] }` fallback dies.

### 1.2 Enum tagging rule

The Rust side should mirror the TS discriminated-union style exactly:

- **internally tagged enums**
- `#[serde(tag = "kind", rename_all = "snake_case")]` for combinator-owned enums
- plain snake_case field names for payload structs
- `serde_json::Value` for both `JsonValue` and `JsonSchema`

That choice matches the TS source literally:

- `kind: 'map_effect'`
- `kind: 'prompt_contains'`
- `timeout_ms`
- `entity_type`
- `name_prefix`
- `transport_ref`

Externally tagged serde is the wrong shape. Adjacently tagged serde is the wrong shape. Camel-case wire names are the wrong shape.

### 1.3 Rust `Combinator` enum sketch

The Rust `Combinator` enum should live in `fireline-harness` and mirror `packages/client/src/core/combinator.ts` 1:1:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Combinator {
    Observe { sink: ObserveSinkRef },
    MapEffect {
        rewrite: RewriteSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        when: Option<EffectPattern>,
    },
    AppendToSession {
        project: ProjectSpec,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        when: Option<EffectPattern>,
    },
    Filter {
        when: EffectPattern,
        reject: JsonValue,
    },
    Substitute {
        rewrite: RewriteSpec,
        when: EffectPattern,
    },
    Suspend { reason: SuspendReasonSpec },
    Fanout {
        split: FanoutSplitSpec,
        merge: FanoutMergeSpec,
    },
}
```

The nested enums follow the same rule:

- `EffectPattern` — internally tagged, snake_case kinds
- `RewriteSpec` — internally tagged, snake_case kinds
- `ProjectSpec` — internally tagged, snake_case kinds
- `SuspendReasonSpec` — internally tagged, snake_case kinds
- `ObserveSinkRef` — internally tagged, snake_case kinds
- `FanoutSplitSpec` / `FanoutMergeSpec` — internally tagged, snake_case kinds
- `ContextSourceRef` — internally tagged, snake_case kinds

### 1.4 Variant payloads

The payloads must stay exact to the TS source:

| Type | Variant / fields |
|---|---|
| `EffectPattern` | `any`; `prompt_contains { needle }`; `prompt_matches { regex, flags? }`; `tool_call { name?, name_prefix? }`; `peer_call`; `any_of { patterns }` |
| `RewriteSpec` | `prepend_context { sources }`; `route_to_peer { peer }`; `replace_tool { from, to }`; `text_substitute { from, to }` |
| `ProjectSpec` | `audit_effect`; `durable_trace`; `custom { entity_type }` |
| `SuspendReasonSpec` | `require_approval { scope, matcher?, timeout_ms? }`; `require_budget_refresh`; `wait_for_peer { peer }` |
| `ObserveSinkRef` | `state_stream { entity_type }`; `metrics { name }` |
| `FanoutSplitSpec` | `by_peer_list { peers }` |
| `FanoutMergeSpec` | `first_success`; `all` |
| `ContextSourceRef` | `static_text { text }`; `workspace_file { path }`; `datetime` |

### 1.5 Validation rule

Serde should deserialize the exact TS wire shape; semantic validation happens **after** deserialize, inside the interpreter. That keeps wire parsing honest and avoids silently narrowing the TS algebra at the serde layer.

Examples:

- `substitute + prepend_context` should deserialize, then fail as an unsupported pairing if the interpreter does not define that combination.
- `fanout` should deserialize even before the runtime grows branch/merge support.
- missing optional fields (`matcher`, `timeout_ms`, `flags`, `name`, `name_prefix`, `account`) remain absent, not normalized during parse.

## 2. Nested-Type Inventory

This section names the post-restructure home for each nested type and whether Option C can reuse an existing Rust type or needs a new wire-faithful one.

| Type | Post-restructure home | Reuse or new | Notes |
|---|---|---|---|
| `Combinator` | `fireline-harness` | New | Harness-owned algebra; no current Rust equivalent. |
| `EffectPattern` | `fireline-harness` | New | Current approval/budget matching structs are narrower and component-specific. |
| `RewriteSpec` | `fireline-harness` | New | Current runtime has product-specific component configs, not one cross-cutting rewrite enum. |
| `ProjectSpec` | `fireline-harness` | New | Current audit/trace config structs are component-specific. |
| `SuspendReasonSpec` | `fireline-harness` | New | Current approval gate config is one case, not the full suspend union. |
| `ObserveSinkRef` | `fireline-harness` | New | Current audit tracer config is a sink-specific implementation detail, not the public observe union. |
| `FanoutSplitSpec` | `fireline-harness` | New | No current wire type. |
| `FanoutMergeSpec` | `fireline-harness` | New | No current wire type. |
| `ContextSourceRef` | `fireline-harness` | New | Current `ContextSourceSpec` in the runtime tree is close semantically but camelCase-tagged and tied to the legacy topology config layer. |
| `JsonValue` | `fireline-harness` import of `serde_json::Value` | Reuse external type | Do not wrap it. |
| `JsonSchema` | `fireline-harness` import of `serde_json::Value` | Reuse external type | Same runtime representation as `JsonValue`; schema-ness is by position, not by type. |
| `ToolDescriptor` | `fireline-tools::wire` | New wire type | Existing `fireline-tools::ToolDescriptor` is semantically right but its serde contract is camelCase (`inputSchema`), not the TS snake_case wire (`input_schema`). Keep the existing domain type for descriptor projection; add a wire-faithful mirror plus conversion. |
| `TransportRef` | `fireline-tools::wire` | New wire type | Existing `fireline-tools::TransportRef` serializes `kind: "mcpUrl"` / `peerRuntime` / `inProcess`; TS sends `mcp_url` / `peer_runtime` / `in_process`. |
| `CredentialRef` | `fireline-tools::wire` | New wire type | Existing `fireline-tools::CredentialRef` serializes `oauthToken`; TS sends `oauth_token`. |
| `CapabilityRef` | `fireline-tools::wire` | New wire type | Existing `fireline-tools::CapabilityRef` uses camelCase field names (`transportRef`, `credentialRef`) and therefore cannot be reused as the exact Option C wire type. |
| `ResourceRef` | `fireline-resources::wire` | New wire type | Current `fireline-resources::ResourceRef` is close but not exact: it is camelCase-tagged and does not yet model TS `git_remote.ref`, `git_remote.subdir`, or `read_only`. |
| `Endpoint` | `fireline-runtime::provider_trait` | Reuse existing type | `url` and `headers` already match the TS wire shape; no separate mirror needed. |

### 2.1 Why add `wire` mirrors for tools/resources

The Option C goal is **exact TS parity on the launch wire**, not "close enough to current Rust serde." The current Rust tool/resource types were designed for other contracts:

- tool descriptor projection onto the Session stream
- runtime-internal provider/resource plumbing

Those uses should not force the topology wire to inherit camelCase field names that the TS substrate never declared.

The clean split is:

- `fireline-tools::wire::*` and `fireline-resources::wire::*` mirror the TS launch wire exactly
- existing `fireline-tools::*` and `fireline-resources::*` remain domain/runtime types
- `From` / `TryFrom` conversions bridge wire → domain after deserialize

That avoids breaking existing envelope contracts while making the combinator wire exact.

## 3. Interpreter Shape

### 3.1 Ownership

The interpreter belongs in **`fireline-harness`**, not `fireline-runtime`.

Reason:

- the combinator algebra is a Harness concern
- `fireline-runtime` assembles and launches runtimes, but does not own effect semantics
- `fireline-orchestration` remains separate; suspend combinators surface blocked state and durable wake evidence, but do not require a direct orchestration dependency inside `fireline-harness`

### 3.2 Public trait sketch

The public trait should reduce one combinator at a time into the harness pipeline builder that lives in `fireline-harness`:

```rust
pub trait CombinatorInterpreter {
    type Error;

    fn apply(
        &self,
        combinator: &Combinator,
        pipeline: &mut fireline_harness::pipeline::EffectPipelineBuilder,
    ) -> Result<(), Self::Error>;
}
```

This doc intentionally does **not** define `EffectPipelineBuilder`. It is the post-restructure harness-owned pipeline type that replaces the current public dependence on `TopologyRegistry` / named component factories.

`fireline-runtime` owns the fold:

```text
let mut pipeline = EffectPipelineBuilder::new(...);
for combinator in topology.iter() {
    interpreter.apply(combinator, &mut pipeline)?;
}
let resolved = pipeline.finish()?;
```

### 3.3 Concrete implementation

The concrete implementation should be a single `FirelineCombinatorInterpreter` in `fireline-harness`. Option C explicitly rejects reviving a registry of public wire-level named component strings.

Named implementation types may survive **internally** as helpers:

- approval logic
- budget gate logic
- context gathering
- audit / trace writers
- tool registry / peer routing helpers

But the public launch contract is `Vec<Combinator>`, and the public harness entrypoint is `CombinatorInterpreter`.

### 3.4 Transition rules

The interpreter applies combinators in array order. Each combinator appends one stage to the pipeline:

| Combinator | Interpreter rule |
|---|---|
| `observe` | Add an observer stage. `state_stream` emits side-channel evidence; `metrics` emits metric observations. No effect mutation. |
| `map_effect` | Add a conditional, shape-preserving transformer stage. `when = None` means unconditional. `prepend_context` and text-level rewrites live here. |
| `append_to_session` | Add a conditional session-append stage. `audit_effect` emits audit artifacts, `durable_trace` emits bidirectional trace artifacts, `custom` emits a custom entity type. |
| `filter` | Add a gate stage. When the pattern matches, terminate the current effect with the provided `reject` payload. |
| `substitute` | Add a conditional routing/substitution stage. This is where peer routing and tool substitution live. |
| `suspend` | Add a blocking stage that emits durable suspend evidence and returns a blocked/suspended result to the host/runtime boundary. Orchestration reacts later by waking from durable state; the interpreter itself does not depend on `fireline-orchestration`. |
| `fanout` | Add a branch/merge stage. Split produces child effects, downstream executes them, merge collapses the result set by `first_success` or `all`. |

### 3.5 Semantic pairing rules

Serde should accept the full TS union, but the interpreter should validate pairings explicitly. The rule is "deserialize broadly, interpret narrowly and loudly."

Recommended pairings for the first concrete implementation:

- `map_effect` supports `prepend_context` and `text_substitute`
- `substitute` supports `route_to_peer` and `replace_tool`
- `append_to_session` supports every `ProjectSpec` case
- `filter` treats `reject` as opaque JSON and delegates budget/policy meaning to the stage implementation
- `suspend` supports every `SuspendReasonSpec` case, even if some reduce to "blocked until durable external event"

Any unsupported pairing should fail the runtime build with a typed interpreter error, not a translator fallback and not a silent no-op.

### 3.6 Relation to current internals

Option C does **not** require deleting the current implementation code that already exists behind named components. It does require moving the boundary:

- current component implementations become interpreter internals
- the old public wire layer (`TopologySpec { components: [...] }`) stops being the runtime contract
- `TopologyRegistry` stops being the launch-time public abstraction

## 4. Migration Plan

These PRs land **after** the crate restructure in `docs/proposals/crate-restructure-manifest.md`.

### PR 1 — Add wire-faithful combinator and nested types

Scope:

- add `fireline-harness::combinator::{Combinator, EffectPattern, ...}`
- add `fireline-tools::wire::{ToolDescriptor, TransportRef, CredentialRef, CapabilityRef}`
- add `fireline-resources::wire::ResourceRef`
- add pure serde round-trip tests for the exact TS wire examples

No runtime assembly changes yet.

Rollback:

- revert the new wire modules and tests only
- no launch path changes, so rollback is mechanical

### PR 2 — Add `CombinatorInterpreter` and concrete harness reduction

Scope:

- add `CombinatorInterpreter`
- add `FirelineCombinatorInterpreter`
- add pipeline-builder adapters in `fireline-harness`
- exercise reduction rules against `fireline-semantics` fixtures or harness-local pure tests

The legacy runtime launch path still builds through the old topology path.

Rollback:

- revert interpreter and builder-adapter files
- PR 1 wire types remain inert and harmless

### PR 3 — Switch `fireline-runtime` launch spec to combinator topology

Scope:

- change the runtime launch request shape so `topology` deserializes as `Vec<Combinator>`
- thread parsed combinators through persisted runtime spec/load paths
- build the harness pipeline by folding `CombinatorInterpreter` over the parsed topology

Compatibility rule:

- if short-lived backward compatibility is required for already-persisted runtime specs, it may exist only as a **temporary compatibility decoder inside `fireline-runtime`**
- it must not become a reusable translator module and must not synthesize new product-level helpers

Rollback:

- revert the runtime launch-spec change and restore the previous topology field type
- PRs 1–2 remain safely unused

### PR 4 — Remove the legacy named-component launch contract

Scope:

- delete `TopologySpec { components }` from the external runtime launch API
- remove public `TopologyComponentSpec` parsing from runtime creation/persistence
- keep any useful helper implementations internal to `fireline-harness`, behind the interpreter

Rollback:

- revert PR 4 alone; PR 3 still provides the combinator path

### PR 5 — Flip the TS Fireline host to array-only topology

Scope:

- remove the `{ components: [] }` fallback from the Fireline host satisfier
- serialize empty topology as `[]`
- update docs/examples to treat array topology as the only wire shape

Rollback:

- revert the TS-side cleanup while keeping the Rust combinator path intact

## 5. Deprecation of the TS Translator

`packages/client/src/host-fireline/topology-translator.ts` stays deleted.

Option C closes the door on that pattern permanently:

- no TS-side combinator → legacy topology adapter
- no new public Rust-side translator module as a substitute
- no product code depending on `{ name, config }` launch payloads as the long-term contract

If temporary backward compatibility for persisted specs is needed during rollout, it is a **short-lived runtime compatibility decoder**, not a translator architecture and not a new user-facing surface.

The permanent public contract is:

```text
topology: Vec<Combinator>
```

## Open Questions

1. **Tool/resource wire mirrors vs dual-serde domain types.** This proposal leans toward dedicated `wire` modules because the current domain types already serve other contracts with different field naming. That is the cleanest design, but it is still a choice.
2. **`fanout` first landing.** The wire type is straightforward; the execution path depends on whether `fireline-harness` grows true branch/merge support in the same PR or lands `fanout` as an explicit interpreter error until that builder support exists.
3. **Temporary persisted-spec compatibility.** If existing stored runtime specs still carry legacy `{ components }`, the rollout needs a bounded migration strategy. This proposal deliberately does not invent that migration mechanism; it only states that the mechanism must not become a permanent translator layer.
