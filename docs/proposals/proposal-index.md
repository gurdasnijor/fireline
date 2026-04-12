# Proposal Index and Canonical-Identifier Consistency Audit

> Status: active index
> Date: 2026-04-12
> Scope: `docs/proposals/*`, plus relevant supporting docs in `docs/explorations/` and `docs/demos/`

## Executive Summary

- Audited all 21 files in `docs/proposals/`.
- Active drift against `acp-canonical-identifiers.md`: `durable-subscriber.md`, `platform-sdk-api-design.md`, `client-api-redesign.md`, `unified-materialization.md`, and `secrets-injection-component.md`.
- `webhook-support.md` is superseded by `durable-subscriber.md` §5.2 "Webhook Delivery Profile".
- Normative but already aligned: `acp-canonical-identifiers.md`, `acp-canonical-identifiers-execution.md`, and `acp-canonical-identifiers-verification.md` contain forbidden tokens only in removal, migration, or verification contexts.
- Historical-only hits: `runtime-host-split.md` and `crate-restructure-manifest.md` mention old synthetic-id machinery, but both are explicitly superseded.

## 1. Audit Outcome

### 1.1 Active proposals aligned with the canonical-identifier bar

No synthetic-id drift found in target design:

- `cross-host-discovery.md`
- `resource-discovery.md`
- `deployment-and-remote-handoff.md`
- `declarative-agent-api-design.md`
- `durable-promises.md`
- `sandbox-provider-model.md`
- `stream-fs-spike.md`
- `competitive-analysis-anthropic-managed-agents.md`

Normative and aligned by construction:

- `acp-canonical-identifiers.md`
- `acp-canonical-identifiers-execution.md`
- `acp-canonical-identifiers-verification.md`

### 1.2 Historical-only references: no rewrite required

These docs contain hits, but only as history or superseded architecture:

- `runtime-host-split.md` — historical references to `ActiveTurnIndex`
- `crate-restructure-manifest.md` — historical file-move reference to `child_session_edge`
- `client-primitives.md`
- `fireline-host-audit.md`
- `fireline-host-cleanup-plan.md`
- `option-c-combinator-serde.md`

### 1.3 Drift findings

| Proposal | Lines | Priority | Drift | Canonical replacement |
|---|---:|---|---|---|
| `durable-subscriber.md` | `66-70`, `154-157`, `321-327`, `393-401`, `447` | Critical | `CrossSessionKey` and `cross_session` model cross-session lineage as a Fireline completion key | Completion identity stays caller-local: `PromptKey(SessionId, RequestId)` or `ToolKey(SessionId, ToolCallId)`. Cross-session causality lives only in ACP `_meta` trace context and the trace backend. |
| `platform-sdk-api-design.md` | `114-115`, `151-198`, `215`, `395-402` | Critical | Public TS API still types ACP ids as `string` and exposes `logicalConnectionId`, `PromptTurnRow`, `childSessionEdges`, `ConnectionRow`, `TerminalRow`, and `RuntimeInstanceRow` in `fireline.db()` | Use `@agentclientprotocol/sdk` branded types, rename prompt-turn surface to prompt-request, and keep `fireline.db()` agent-plane only. Move infra state to admin APIs. |
| `client-api-redesign.md` | `190`, `363`, `422`, `437`, `442-475` | Critical | Topology examples still claim `child_session_edge` rows and one tenant stream as the lineage/visibility model | Cross-agent causality is OTel trace context via ACP `_meta`; state examples should use prompt requests and agent-plane collections, not edge rows. |
| `unified-materialization.md` | `14`, `89-100` | Design | Treats `ActiveTurnIndex` and `prompt_turn` as steady-state projection concepts | Rewrite around `SessionIndex`, `HostIndex`, and ACP-keyed prompt/permission/tool-call projections. Mark `ActiveTurnIndex` transitional and deleted by canonical-identifiers Phase 5. |
| `secrets-injection-component.md` | `147`, `531` | Design | Proposed Rust types still use `String` for `session_id` in session-scoped keys and agent-plane audit events | Type `session_id` as `sacp::schema::SessionId`. |

### 1.4 Relevant non-proposal drift

- `docs/explorations/managed-agents-mapping.md:231` still describes `ActiveTurnIndex` as part of the live substrate. This is low-priority exploration cleanup, not a proposal blocker.
- `docs/demos/pi-acp-to-openclaw.md` did not contain canonical-identifier drift.

## 2. Proposal Graph

### 2.1 Root identity and verification chain

- `acp-canonical-identifiers.md`
  - root contract for agent-plane identity, plane separation, and type-enforced ACP ids
- `acp-canonical-identifiers-execution.md`
  - rollout plan for the canonical-id cut
- `acp-canonical-identifiers-verification.md`
  - invariant, fixture, grep-audit, and TLA/Stateright plan for the cut

### 2.2 Durable workflow chain

- `durable-subscriber.md`
  - depends on `acp-canonical-identifiers.md`
- `durable-promises.md`
  - depends on `durable-subscriber.md`
- `webhook-support.md`
  - superseded by `durable-subscriber.md` §5.2 "Webhook Delivery Profile"

### 2.3 Client and SDK chain

- `client-api-redesign.md`
  - owns declarative composition: `compose`, `sandbox`, `middleware`, `agent`, `peer`, `fanout`, `pipe`
- `platform-sdk-api-design.md`
  - depends on `client-api-redesign.md` for composition and on `acp-canonical-identifiers.md` for identifier and state-shape correctness
- `declarative-agent-api-design.md`
  - depends on `deployment-and-remote-handoff.md`, `platform-sdk-api-design.md`, and `sandbox-provider-model.md`

### 2.4 Deployment and infrastructure chain

- `deployment-and-remote-handoff.md`
  - DX and deployment root for local-vs-remote execution
- `sandbox-provider-model.md`
  - depends on `deployment-and-remote-handoff.md`, `cross-host-discovery.md`, and `resource-discovery.md`
- `secrets-injection-component.md`
  - depends on `deployment-and-remote-handoff.md`
- `cross-host-discovery.md`
  - infrastructure discovery plane for hosts/sandboxes
- `resource-discovery.md`
  - infrastructure discovery plane for resources
- `stream-fs-spike.md`
  - narrow implementation spike on top of `resource-discovery.md` and `cross-host-discovery.md`

### 2.5 Read-model and analysis docs

- `unified-materialization.md`
  - should depend explicitly on `acp-canonical-identifiers.md` because it defines the steady-state read-model surface
- `competitive-analysis-anthropic-managed-agents.md`
  - strategic analysis document; consumes the other active proposals but is not itself a design prerequisite

## 3. Active Proposal Catalog

| Proposal | Summary | Depends on | Audit result |
|---|---|---|---|
| `acp-canonical-identifiers.md` | Makes ACP ids and ACP `_meta` trace context the only canonical agent-plane identity surface. | — | Root; aligned |
| `acp-canonical-identifiers-execution.md` | Breaks the canonical-id cut into phased, revertable rollout steps. | `acp-canonical-identifiers` | Aligned |
| `acp-canonical-identifiers-verification.md` | Defines the grep, fixture, TLA, and test gates for the canonical-id cut. | `acp-canonical-identifiers` | Aligned |
| `client-api-redesign.md` | Defines the declarative composition model for `@fireline/client`. | composition root | Drift: state/topology sections |
| `competitive-analysis-anthropic-managed-agents.md` | Strategic comparison of Fireline versus Anthropic Managed Agents. | multiple active proposals | Aligned |
| `cross-host-discovery.md` | Moves host/sandbox discovery onto durable-streams infrastructure streams. | deployment posture | Aligned |
| `declarative-agent-api-design.md` | CLI and authoring DX proposal for `fireline run` / `fireline deploy`. | deployment, platform SDK, provider model | Aligned |
| `deployment-and-remote-handoff.md` | Defines local-first authoring and remote deployment posture. | composition + DX root | Aligned |
| `durable-promises.md` | Imperative awakeable API layered over passive durable subscribers. | `durable-subscriber` | Aligned, but inherits subscriber key surface |
| `durable-subscriber.md` | General durable workflow primitive for approvals, webhooks, timers, and integrations. | `acp-canonical-identifiers` | Drift: `CrossSessionKey` |
| `platform-sdk-api-design.md` | Imperative SDK for apps, dashboards, and bots on top of Fireline. | canonical ids + client composition | Drift: DB shape and type surface |
| `resource-discovery.md` | Moves resource publication and lookup onto durable-stream streams. | cross-host discovery | Aligned |
| `sandbox-provider-model.md` | Unifies local, Docker, remote, and API-backed sandboxes behind one provider interface. | deployment, discovery, resource discovery | Aligned |
| `secrets-injection-component.md` | Harness-level secret resolution and injection model. | deployment | Drift: plain `String` session ids |
| `stream-fs-spike.md` | Narrow spike for `StreamFs` as a discoverable resource mount. | resource discovery, cross-host discovery | Aligned |
| `unified-materialization.md` | Shared durable-stream projection abstraction for read models. | should depend on canonical ids | Drift: `ActiveTurnIndex` target model |
| `webhook-support.md` | Historical webhook-forwarding proposal. | durable stream subscription substrate | Superseded by `durable-subscriber.md` §5.2 |

## 4. Merge and Deprecation Recommendations

### 4.1 `webhook-support.md` is superseded

Recommendation: keep `webhook-support.md` only as a historical reference and treat `durable-subscriber.md` §5.2 "Webhook Delivery Profile" as the live source of truth.

Reason:

- the mechanism is now the same substrate
- keeping a separate webhook-specific primitive doc encourages architectural drift
- only the delivery-specific material is unique: retry policy, cursor persistence, and webhook payload shape

### 4.2 Keep `client-api-redesign.md`, but narrow its scope

Recommendation: keep `client-api-redesign.md` as the declarative composition doc, but remove ownership of state-plane semantics that now belong to `platform-sdk-api-design.md` and `acp-canonical-identifiers.md`.

Reason:

- the composition model is still valid
- the state-observation examples and lineage language are not

### 4.3 Keep historical docs archived as-is

No rewrite needed for:

- `client-primitives.md`
- `crate-restructure-manifest.md`
- `fireline-host-audit.md`
- `fireline-host-cleanup-plan.md`
- `option-c-combinator-serde.md`
- `runtime-host-split.md`

They should remain clearly labeled historical/superseded, but they do not need canonical-id patch work.

## 5. Patch Queue

### 5.1 `durable-subscriber.md` — Critical

- `66-70`
  Replace the peer-routing row with:
  `| Peer routing | prompt or tool event | peer delivery acknowledgment on the caller stream | PromptKey(SessionId, RequestId) or ToolKey(SessionId, ToolCallId) |`
- `154-157`
  Replace:
  `pub enum CompletionKey { PromptKey(...), ToolKey(...), CrossSessionKey(...) }`
  with:
  `pub enum CompletionKey { PromptKey(SessionId, RequestId), ToolKey(SessionId, ToolCallId) }`
- `321-327`, `393-401`, `447`
  Delete `CrossSessionKey` / `cross_session` language and replace with:
  `Cross-session causality is not a completion key. Peer-call lineage is carried only by ACP _meta trace context and queried in the trace backend. Subscriber completion identity remains caller-local.`
- Priority: `critical`

### 5.2 `platform-sdk-api-design.md` — Critical

- `108-116`
  Replace `resolvePermission(sessionId: string, requestId: string, ...)` with `resolvePermission(sessionId: SessionId, requestId: RequestId, ...)` and import ACP types from `@agentclientprotocol/sdk`.
- `151-198`
  Replace the `FirelineViews` / `FirelineDB` block with an agent-plane-only surface. Exact replacement:

  ```ts
  import type { RequestId, SessionId } from '@agentclientprotocol/sdk'
  import type { ChunkRow, PermissionRow, PromptRequestRow, SessionRow } from './types'

  export interface FirelineViews {
    pendingPermissions(): ObservableCollection<PermissionRow>
    sessionRequests(sessionId: SessionId): ObservableCollection<PromptRequestRow>
    requestChunks(sessionId: SessionId, requestId: RequestId): ObservableCollection<ChunkRow>
    sessionPermissions(sessionId: SessionId): ObservableCollection<PermissionRow>
  }

  export interface FirelineDB {
    readonly sessions: ObservableCollection<SessionRow>
    readonly promptRequests: ObservableCollection<PromptRequestRow>
    readonly permissions: ObservableCollection<PermissionRow>
    readonly chunks: ObservableCollection<ChunkRow>
    readonly views: FirelineViews
    readonly collections: {
      readonly sessions: ObservableCollection<SessionRow>
      readonly promptRequests: ObservableCollection<PromptRequestRow>
      readonly permissions: ObservableCollection<PermissionRow>
      readonly chunks: ObservableCollection<ChunkRow>
    }
    preload(): Promise<void>
    close(): void
  }
  ```

- `215-230`
  Add:
  `fireline.db()` exposes only agent-plane rows. Host, sandbox, provider, connection, and terminal inventory belongs to operator/admin APIs, not the public DB surface.
- `395-402`
  Replace `createSessionTurnsCollection` / `db.views.sessionTurns` with `createSessionRequestsCollection` / `db.views.sessionRequests`.
- Priority: `critical`

### 5.3 `client-api-redesign.md` — Critical

- `190`
  Replace:
  `// the stream carries cross-agent lineage (child_session_edge events)`
  with:
  `// cross-agent causality is visible through ACP _meta trace context and the trace backend`
- `363`, `437`
  Replace `db.collections.promptTurns` with `db.promptRequests` or `db.collections.promptRequests`.
- `422`
  Replace:
  `// reviewer runs first; writer picks up from the shared tenant stream`
  with:
  `// reviewer runs first; writer observes the prior agent-plane events through Fireline's state substrate`
- `442-475`
  Replace the entire subsection with:
  `Multi-agent topologies expose deployment-wide agent-plane visibility through Fireline state. Dashboards see sessions, prompt requests, permissions, and chunks across the deployment. Cross-agent causality is not materialized as childSessionEdges; it is queried through W3C trace context and the configured trace backend.`
- Priority: `critical`

### 5.4 `unified-materialization.md` — Design

- `12-15`
  Replace:
  `SessionIndex, HostIndex, and ActiveTurnIndex now implement ...`
  with:
  `SessionIndex, HostIndex, and future ACP-keyed prompt/permission/tool-call projections implement ...`
- `89-100`
  Replace the `ActiveTurnIndex` paragraph with:
  `No steady-state projection should depend on prompt_turn or other synthetic turn ids. Any temporary waiter coordination must be keyed by canonical ACP identifiers and treated as transitional pending the canonical-identifiers execution plan.`
- Priority: `design`

### 5.5 `secrets-injection-component.md` — Design

- `145-149`
  Replace:
  `struct SessionCacheKey { session_id: String, ... }`
  with:
  `struct SessionCacheKey { session_id: SessionId, ... }`
  and add `use sacp::schema::SessionId`.
- `529-535`
  Replace:
  `session_id: String`
  with:
  `session_id: SessionId`
- Priority: `design`

### 5.6 `webhook-support.md` — SUPERSEDED

- Status:
  superseded by `durable-subscriber.md` §5.2 "Webhook Delivery Profile"
- Historical handling:
  retain the file with a superseded banner only; do not continue editing it as a live design doc
- Priority:
  complete

## 6. Suggested Dispatch Order

1. `durable-subscriber.md`
2. `platform-sdk-api-design.md`
3. `client-api-redesign.md`
4. `unified-materialization.md`
5. `secrets-injection-component.md`
6. `webhook-support.md` supersession cleanup

That order keeps the root identity and workflow substrate ahead of the client-surface rewrites that depend on them.
