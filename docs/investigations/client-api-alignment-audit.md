# Client API Alignment Audit

Date: 2026-04-11

Scope:
- Every file under `packages/client/src/`
- Alignment against `docs/proposals/client-api-redesign.md`
- Additional emphasis from the current target contract:
  - Fireline does not wrap ACP
  - `@fireline/state` is the only observation/subscription interface
  - No side channels outside control plane, ACP session plane, and stream-db observation plane
  - Public client surface is `Sandbox`, `compose`, `agent`, `sandbox`, middleware helpers, `SandboxAdmin`, and types

## Summary

Verdict counts across `packages/client/src/`:
- `KEEP`: 4 files
- `REFACTOR`: 5 files
- `DELETE`: 10 files
- `UNKNOWN`: 0 files

Highest-signal misalignments:
- ACP is still wrapped in three files plus a browser convenience client.
- The package still contains old side-channel surfaces (`core`, `orchestration`, `sandbox-local`) that bypass the proposal's narrow control/session/observation split.
- The new `Sandbox` client still contains a legacy `/v1/runtimes` fallback.
- `topology.ts` and `catalog.ts` keep older substrate abstractions alive that are outside the minimal client contract.

## File-by-file verdicts

| File | Verdict | Why |
|---|---|---|
| `packages/client/src/acp-core.ts` | `DELETE` | Directly violates the proposal. It wraps `ClientSideConnection`, exports Fireline-specific ACP connection types, and adds an `updates()` queue abstraction on top of ACP. The redesign says users import `@agentclientprotocol/sdk` directly and Fireline never wraps ACP. |
| `packages/client/src/acp.ts` | `DELETE` | Node ACP transport wrapper over WebSocket plus `connectAcp()` helper. This is exactly the kind of ACP wrapper the proposal says should not exist. |
| `packages/client/src/acp.browser.ts` | `DELETE` | Browser ACP wrapper mirroring `acp.ts`. Same violation as above, just in browser form. |
| `packages/client/src/admin.ts` | `KEEP` | Aligned with the proposed operator surface. It exposes `get`, `list`, `destroy`, `status`, and `healthCheck` against `/v1/sandboxes` and stays on the control plane. |
| `packages/client/src/browser.ts` | `DELETE` | `createBrowserFirelineClient()` bundles ACP connection, topology builder access, and stream-db access into a convenience client. That reintroduces side-channel convenience APIs the proposal explicitly removes. |
| `packages/client/src/catalog.ts` | `REFACTOR` | Potentially useful, but not part of the minimal client contract. Agent catalog lookup/resolution is a fourth concern beyond control/session/observation. If retained, it should move out of `@fireline/client` into a separate higher-level package or tool layer. |
| `packages/client/src/control-plane.ts` | `KEEP` | Small internal helper for control-plane HTTP requests. This stays within the proposal's control plane and does not introduce extra abstractions. |
| `packages/client/src/core/combinator.ts` | `DELETE` | Legacy substrate modeling layer with `Combinator`, `Topology`, `approvalGate`, `parallelPeers`, and related helpers. This is a separate abstraction system that conflicts with the redesign's narrow serializable `compose`/middleware surface. |
| `packages/client/src/core/index.ts` | `DELETE` | Barrel for the old `core` substrate surface. If `core/combinator.ts` and `core/tool.ts` go away, this barrel should go too. |
| `packages/client/src/core/resource.ts` | `REFACTOR` | The resource reference types themselves are still conceptually useful, but the proposal wants a dedicated `@fireline/client/resources` surface with thin constructors, not a legacy `core` barrel. The type definitions should move/re-export under the new resource-oriented module shape. |
| `packages/client/src/core/tool.ts` | `DELETE` | Legacy Tools-primitive modeling (`ToolDescriptor`, `CapabilityRef`, transports, credentials). That surface is outside the new minimal client contract and couples the client to older substrate abstractions. |
| `packages/client/src/index.ts` | `KEEP` | The root file is close to the intended direction: it exports `Sandbox`, `compose`, `agent`, `sandbox`, and types. The file itself is aligned, even though the overall package still exposes other legacy subpaths. |
| `packages/client/src/middleware.ts` | `KEEP` | Aligned with the redesign. It exports serializable middleware helpers (`trace`, `approve`, `budget`, `contextInjection`, `peer`) and does not wrap ACP. |
| `packages/client/src/orchestration/index.ts` | `DELETE` | Adds orchestrator APIs (`whileLoopOrchestrator`, cron/http orchestration) that create another plane outside the redesign's control/session/observation split. Not part of the minimal client contract. |
| `packages/client/src/sandbox-local/client.ts` | `DELETE` | A direct local subprocess sandbox surface with `provision`, `execute`, `status`, and `stop` bypasses the Fireline control plane entirely. This is a side channel and conflicts with the proposal's single control-plane provisioning path. |
| `packages/client/src/sandbox-local/index.ts` | `DELETE` | Barrel for the side-channel local sandbox client. Should disappear with `sandbox-local/client.ts`. |
| `packages/client/src/sandbox.ts` | `REFACTOR` | Mostly points in the right direction, but the file still falls back to deleted `/v1/runtimes` behavior and silently drops parts of `SandboxDefinition` (`envVars`, `provider`, `labels`, `image`) when building the request. The legacy endpoint fallback in particular is out of alignment with the redesigned `/v1/sandboxes` contract. |
| `packages/client/src/topology.ts` | `REFACTOR` | Public `TopologyBuilder` is a leftover side abstraction. The redesign pushes composition through `compose` plus topology operators, not an explicit builder object. The `TopologySpec` type may remain useful internally, but the builder should be removed or hidden. |
| `packages/client/src/types.ts` | `REFACTOR` | This is the right general area, but it still depends on legacy modules (`core/resource`, `topology`) and its shapes do not fully match the proposal's more explicit middleware-chain and topology story. The public type surface should be made self-contained and decoupled from legacy support files. |

## Specific requested checks

### `packages/client/src/acp-core.ts`

Verdict: `DELETE`

Reason:
- Exports Fireline-owned ACP connection abstractions (`OpenAcpConnection`, `AcpSocketHandle`, `AcpConnectOptions`, `AcpInitializeOptions`)
- Wraps `ClientSideConnection`
- Queues `SessionNotification` updates in Fireline-owned infrastructure

This is the clearest direct violation of "Fireline never wraps ACP."

### `packages/client/src/acp.ts`

Verdict: `DELETE`

Reason:
- Wraps ACP over Node WebSocket transport
- Re-exports ACP wrapper types from `acp-core.ts`
- Adds a Fireline-branded `connectAcp()` helper the proposal says should not exist

### `packages/client/src/acp.browser.ts`

Verdict: `DELETE`

Reason:
- Browser twin of `acp.ts`
- Same wrapper violation, same non-proposal surface

### `packages/client/src/browser.ts`

Verdict: `DELETE`

Reason:
- Adds a `BrowserFirelineClient` facade that bundles ACP, topology, and state access
- Reintroduces a Fireline-managed ACP connection surface
- Conflicts with the proposal's explicit separation: Fireline for control plane, ACP SDK for session plane, `@fireline/state` for observation

### `packages/client/src/host.ts`

Verdict: already deleted

Reason:
- `packages/client/src/host.ts` is absent from the tree, which is aligned with the redesign away from the legacy host/runtime client surface.

### `packages/client/src/topology.ts`

Verdict: `REFACTOR`

Reason:
- `TopologyBuilder` is not the target public composition model
- The type-level composition story in the proposal is `compose` plus topology operators, not an imperative builder
- The low-level `TopologySpec` type may still be useful as an internal transport shape

### `packages/client/src/catalog.ts`

Verdict: `REFACTOR`

Reason:
- Not obviously wrong, but clearly outside the minimal contract
- Catalog/registry resolution introduces an extra concern absent from the proposal's export list
- If product still wants it, it should likely move to a separate package or app-layer utility

### ACP re-exports / ACP wrappers

Files that wrap or re-export ACP-adjacent surfaces:
- `packages/client/src/acp-core.ts`
- `packages/client/src/acp.ts`
- `packages/client/src/acp.browser.ts`
- `packages/client/src/browser.ts`

All four are misaligned with the proposal and should be removed.

## Package-level notes outside `src/`

Not part of the requested file-by-file scan, but materially relevant:
- `packages/client/package.json` still exports legacy/misaligned subpaths: `./browser`, `./core`, `./orchestration`, and `./sandbox-local`
- Even if `index.ts` is aligned, those export-map entries keep the old side-channel API alive for consumers

## Recommended cut order

1. Remove ACP wrappers and the browser convenience client:
   `acp-core.ts`, `acp.ts`, `acp.browser.ts`, `browser.ts`
2. Remove side-channel substrate surfaces:
   `sandbox-local/*`, `orchestration/index.ts`, `core/combinator.ts`, `core/tool.ts`, `core/index.ts`
3. Refactor remaining support files into the minimal contract:
   `sandbox.ts`, `types.ts`, `topology.ts`, `catalog.ts`, `core/resource.ts`
4. Trim package exports so only the intended public surface remains
