# Kill `routes_acp.rs`: rebase Fireline middleware onto SDK `ConductorImpl` + `Proxy` primitives

**Status:** Proposal
**Author:** @gurdasnijor
**Date:** 2026-04-13
**Bead:** `mono-u2y`
**Partner (TS):** `mono-00d` — `Conductor<Name, Role>.connect_to(transport)` (`docs/proposals/harness-conductor-connect-to.md`)
**Prerequisite:** `mono-5h6` — server-side conductor/transport decoupling (in flight)

---

## TL;DR

`crates/fireline-harness/src/routes_acp.rs` is a hand-rolled parallel implementation of things the ACP Rust SDK already provides as first-class types:

- `ConductorImpl::new_proxy` + `InstantiateProxies` — declarative component graph construction
- `ConnectTo<Host>` — transport-agnostic terminator trait
- `Proxy` role marker + `acp.Client + acp.Agent` dual-role trait — what a middleware component *is*

This proposal deletes the bespoke wiring in `routes_acp.rs` and rebases Fireline's middleware (trace, approve, peer_routing, secrets_injection, telegram, webhook, autoApprove, budget, context_injection, attach_tool) onto those SDK primitives. The result: Fireline speaks ACP through the supported vocabulary, not through a parallel interpretation of it.

This is the Rust-side pair to `mono-00d`. Both sides of the protocol land on the same construction+transport shape: **`Conductor::new_proxy(...).connect_to(transport)`**.

## Motivation

### `routes_acp.rs` today entangles too many concerns

The current file (~hundreds of LOC) does all of:

1. **Axum WebSocket upgrade** (`handle_upgrade`) — HTTP server concern.
2. **Component instantiation** — hand-threaded construction of every middleware:
   - `audit` / `trace`
   - `approval_gate`
   - `peer_mcp` / `peer_routing`
   - `secrets_injection`
   - `telegram`
   - `webhook_subscriber`
   - `auto_approve`
   - `budget`
   - `context_injection`
   - `attach_tool`
   - `always_on_deployment`
3. **Session binding + canonical-id bookkeeping** — ACP protocol concern.
4. **CORS + bearer auth** (`mono-uc0`, #53) — HTTP transport concern.

Four concerns, one file. `mono-5h6` already made the first cut by extracting `wire_conductor(app_state)` from `handle_upgrade`. This proposal finishes the job: the wiring step itself stops being bespoke Fireline code and becomes a proper `InstantiateProxies` implementation on top of SDK types.

### The SDK already models every concept Fireline needs

| Fireline concept (today) | SDK primitive (what we should use) |
|---|---|
| `TopologyComponentSpec` + `topology.rs` dispatch | `InstantiateProxies::instantiate` — the canonical SDK entry point for "given this initialize request, here's the component list" |
| `routes_acp::wire_conductor` (post-mono-5h6) | `ConductorImpl::<Proxy>::new_proxy(name, instantiator, mcp_bridge_mode)` |
| Middleware structs (e.g. `ApprovalGateComponent`, `TelegramSubscriberComponent`) | Types implementing both `acp.Client` *and* `acp.Agent` per the [ACP Proxy Chains RFD](https://agentclientprotocol.com/rfds/proxy-chains) |
| `handle_upgrade` axum WS terminator | `ConnectTo<Client>` adapter that `conductor.connect_to(transport)` terminates onto |
| `mono-5h6` `serve_stdio(app)` | `sacp_tokio::Stdio::new()` (already used by `src/bin/testy_load.rs`) |
| Canonical serve pattern | [`agent-client-protocol-core/src/mcp_server/server.rs#L206`](https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-core/src/mcp_server/server.rs#L206) |

Right-hand column exists and is stable. Left-hand column is parallel reinvention.

## Proposed architecture

### Layer cake after this bead lands

```
+--------------------------------------------------------+
|  fireline-host                                         |
|                                                        |
|   pub struct Conductor { inner: ConductorImpl<Proxy> } |
|   impl Conductor {                                     |
|     pub fn new(spec: HostSpec) -> Self { ... }         |
|     pub async fn connect_to(                           |
|       self,                                            |
|       transport: impl ConnectTo<Client>,               |
|     ) -> Result<()>                                    |
|   }                                                    |
|                                                        |
|   mod transports {                                     |
|     pub struct AxumWs(WebSocket);                      |
|     impl ConnectTo<Client> for AxumWs { ... }          |
|     pub use sacp_tokio::Stdio;                         |
|   }                                                    |
+--------------------------------------------------------+
                          ^
                          |
+--------------------------------------------------------+
|  fireline-harness (shrinks drastically)                |
|                                                        |
|   pub struct FirelineInstantiator { spec: TopoSpec }   |
|   impl InstantiateProxies for FirelineInstantiator {   |
|     async fn instantiate(                              |
|       &self, cx, tx, init_req,                         |
|     ) -> (InitializeRequest, Vec<Component>) { ... }   |
|   }                                                    |
|                                                        |
|   mod components {                                     |
|     pub struct ApprovalGateProxy { ... }               |
|     impl acp::Client for ApprovalGateProxy { ... }     |
|     impl acp::Agent  for ApprovalGateProxy { ... }     |
|     // ...trace, peer_routing, telegram, webhook, etc. |
|   }                                                    |
+--------------------------------------------------------+
                          ^
                          |
+--------------------------------------------------------+
|  src/main.rs (axum Router mount — ~30 LOC)             |
|                                                        |
|   Router::new()                                        |
|     .route("/acp", get(upgrade))                       |
|     .layer(cors_layer())                               |
|     .layer(bearer_auth_layer())                        |
|                                                        |
|   async fn upgrade(State(inst): State<Arc<...>>) {     |
|     ws.on_upgrade(|socket| async move {                |
|       let conductor = Conductor::new_proxy(inst);      |
|       conductor.connect_to(AxumWs(socket)).await       |
|     })                                                 |
|   }                                                    |
+--------------------------------------------------------+
```

`crates/fireline-harness/src/routes_acp.rs` either:
- **(a) disappears entirely**, with its contents redistributed into `fireline_host::transports::ws` + `fireline_harness::components::*` + `fireline_harness::FirelineInstantiator`, or
- **(b) survives as a ~30-LOC axum Router mount shim** that holds the CORS/bearer layers and calls into `Conductor::new_proxy(inst).connect_to(AxumWs(socket))`.

Either is acceptable. Preference is (a) — moving the axum route mount into `src/main.rs` or `fireline_host::transports::ws` is cleaner, since it puts transport concerns with transport code.

### Each middleware becomes a proper ACP `Proxy`

Per the [ACP Proxy Chains RFD](https://agentclientprotocol.com/rfds/proxy-chains), a proxy component implements **both** `acp::Client` and `acp::Agent` — it looks like an `Agent` to the client upstream of it and like a `Client` to the agent downstream. That dual-role interface is what lets the chain compose.

Example sketch (approval gate):

```rust
pub struct ApprovalGateProxy {
    upstream_tx: mpsc::Sender<ClientDispatch>,
    downstream_tx: mpsc::Sender<AgentDispatch>,
    policies: Vec<ApprovalPolicy>,
    timeout_ms: Option<u64>,
}

#[async_trait]
impl acp::Agent for ApprovalGateProxy {
    // Called by the upstream client.
    async fn prompt(&self, req: PromptRequest) -> Result<PromptResponse, acp::Error> {
        // policy check, wait for durable approval via DurableSubscriber, forward downstream
        ...
        self.downstream_tx.send(AgentDispatch::Prompt(req)).await?;
        ...
    }
    // ...rest of Agent
}

#[async_trait]
impl acp::Client for ApprovalGateProxy {
    // Called by the downstream agent.
    async fn request_permission(
        &self,
        req: RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse, acp::Error> {
        // enqueue on approval subscriber, await resolution, forward upstream
        ...
    }
    // ...rest of Client
}
```

Today the approval-gate logic lives in `crates/fireline-harness/src/approval.rs` wired manually through `routes_acp.rs`. After this bead: the logic is unchanged, the wiring is SDK-driven.

### Migration table (what moves where)

| Today (`routes_acp.rs` + siblings) | After `mono-u2y` |
|---|---|
| `handle_upgrade(conductor, socket)` | `fireline_host::transports::ws::AxumWs` impls `ConnectTo<Client>` |
| `wire_conductor(app_state)` (mono-5h6 output) | `FirelineInstantiator::instantiate` |
| Per-middleware construction blocks in `wire_conductor` | `fireline_harness::components::*` structs, each a `Proxy` impl |
| CORS + bearer auth (mono-uc0, #53) | `src/main.rs` axum Router middleware layers |
| Session binding | Driven by `ConductorImpl`'s internal session management (SDK handles it) |
| Canonical-id bookkeeping | Driven by the proxy chain's standard ACP message flow (no bespoke code) |
| `serve_stdio(app)` (mono-5h6) | `conductor.connect_to(sacp_tokio::Stdio::new())` |

## Dependency order

```
mono-5h6            mono-00d
   |                    |
   v                    v
[wire_conductor     [TS Conductor
 extracted,          speaks same
 stdio sibling]      vocabulary]
   \                /
    \              /
     v            v
      mono-u2y (THIS)
      rebase onto SDK primitives
```

- `mono-5h6` lands the *extraction*: `wire_conductor` becomes a named function. That's the structural prereq — without it, there's nothing to rebase onto SDK types.
- `mono-00d` lands the *TS symmetry*: once the TS client calls `conductor.connect_to(transport)`, the Rust side doing the same thing closes the loop. Both sides speak the same vocabulary.
- `mono-u2y` (this bead) lands the *Rust semantic alignment*: `wire_conductor` stops being Fireline-specific code and becomes an `InstantiateProxies` implementation on top of `ConductorImpl::new_proxy`.

## Acceptance criteria

1. **Zero bespoke conductor wiring in `crates/fireline-harness`.** The crate's public surface becomes (a) proxy components each implementing `acp::Client + acp::Agent`, and (b) the `FirelineInstantiator` that declares the chain. It stops being a component orchestrator.
2. **Each middleware is a proper ACP `Proxy`.** `trace`, `approve`, `peer_routing`, `secrets_injection`, `telegram`, `webhook`, `autoApprove`, `budget`, `context_injection`, `attach_tool`, `always_on_deployment` all become types implementing the dual-role `Client + Agent` interface.
3. **Conductor construction uses `ConductorImpl::new_proxy`.** Not hand-rolled.
4. **WS and stdio terminators share one code path.** Both call `conductor.connect_to(impl ConnectTo<Client>)`. No parallel `handle_upgrade` / `serve_stdio` branches.
5. **`routes_acp.rs` is either deleted or reduced to a ≤30-LOC axum mount shim** with zero conductor-wiring logic.
6. **All existing integration tests pass unchanged.** `managed-agent-tests`, `session_load_local`, `hosted_runtime`, `mesh_baseline`, `state_fixture_snapshot`, `acp_stdio_roundtrip` (from mono-5h6) — all green with no behavioral changes.
7. **Axum Router mount (CORS + bearer + `/acp` path) lives in `src/main.rs` or `fireline_host::transports::ws`**, not in `fireline-harness`.

## Non-goals

- **No protocol changes.** The ACP wire format is unchanged; this is purely a host-side refactor.
- **No middleware behavior changes.** Approval gate still gates, telegram still notifies, webhook still delivers. Only their wiring shape changes.
- **No durable-streams substrate changes.** `DurableSubscriber`, `DurablePromise`, canonical-id primitives all stay as-is. Proxies use them the same way today's middleware does.
- **No TS-side changes.** That's `mono-00d`. This bead is strictly the Rust counterpart.
- **No deletion of `fireline-harness` as a crate.** The crate stays; its contents change. The current name is still accurate once the contents are proxy components + the instantiator.

## Open questions

1. **`mcp_bridge_mode` choice for `new_proxy`.** Fireline already has `peer_mcp` middleware that does MCP bridging. Does it compose with the SDK's `McpBridgeMode`, or do we keep Fireline's peer bridging outside the SDK's mode selector? Leaning toward using the SDK mode and moving `peer_mcp` onto it.
2. **Where `FirelineInstantiator` lives.** Candidates: `fireline_host::conductor::FirelineInstantiator` (host crate owns the wiring), or `fireline_harness::FirelineInstantiator` (harness crate owns the component registry). Leaning toward the host crate — the instantiator is the "glue" between the host spec and the components.
3. **Secrets-injection boundary.** Current `secrets_injection` mutates the prompt path. As a proper `Proxy`, it intercepts `prompt()` on the agent side. Needs a design pass to ensure secret values never traverse the durable-streams substrate (today's guarantee) after moving onto the SDK trait surface.
4. **Mono-uc0 CORS / bearer layer placement.** Can these stay in `routes_acp.rs`-as-thin-mount, or do they belong in a dedicated `fireline_host::transports::ws::middleware` module? Cosmetic; defer to implementation.

## References

- Current hand-rolled wiring to replace: `crates/fireline-harness/src/routes_acp.rs`
- SDK `ConductorImpl::new_proxy`: https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor.rs
- SDK `InstantiateProxies` + `mcp_bridge/actor.rs` canonical instantiator pattern: https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor/mcp_bridge/actor.rs
- ACP core `Proxy` role marker: https://github.com/agentclientprotocol/rust-sdk/blob/main/src/agent-client-protocol-core/src/concepts/proxies.rs
- ACP Proxy Chains RFD (semantic framing for dual-role `Client + Agent` components): https://agentclientprotocol.com/rfds/proxy-chains
- SDK canonical serve pattern reference: https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-core/src/mcp_server/server.rs#L206
- TS symmetric proposal: `docs/proposals/harness-conductor-connect-to.md` (bead `mono-00d`)
- Structural prereq: bead `mono-5h6`
- ACP transports spec: https://agentclientprotocol.com/protocol/transports
