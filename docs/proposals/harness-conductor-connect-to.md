# `Conductor<Name, Role>.connect_to(transport)` — rename `Harness` → `Conductor` and unify `.start()` + `connectAcp`

**Status:** Proposal
**Author:** @gurdasnijor
**Date:** 2026-04-13
**Bead:** `mono-00d`
**Related:** `mono-5h6` (server-side conductor/transport decoupling), `packages/client/src/sandbox.ts`, ACP Proxy Chains RFD

---

## TL;DR

Today a Fireline client composes a `Harness` with `compose(...)`, then makes **two** conceptually separate calls:

1. `.start({ serverUrl })` — POST to `/v1/sandboxes`, get back an ACP URL + state URL.
2. An independent `connectAcp(handle.acp)` / `use-acp` / raw `WebSocket` to actually speak ACP.

The ACP spec already has one canonical primitive for this: a *conductor* connects to a *transport*. The Rust SDK models this as `ConductorImpl<Host>::connect_to(impl ConnectTo<Host>)`. The TypeScript SDK's `ClientSideConnection(toClient, stream)` / `AgentSideConnection(toAgent, stream)` is the same shape spelled differently — a transport-agnostic constructor that takes a `Stream`.

This proposal makes **two** changes to the Fireline TS client surface:

1. **Rename `Harness` / `HarnessSpec` → `Conductor` / `ConductorSpec`** to match the ACP SDK vocabulary. The thing `compose()` produces is an aggregate of components (sandbox + middleware chain + agent) — which is the literal definition of a Rust `Conductor` (see [`ConductorImpl::new`](https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor.rs#L84-L100)). "Harness" is Fireline-local jargon that obscures the parallel.
2. **Unify `.start()` + `connectAcp` into one call, `conductor.connect_to(transport)`**, matching Rust `ConductorImpl<Host>::connect_to`. Provisioning is hidden behind the transport. The "dual server-side `run_on_ws` / client-side `runOnWs`" framing collapses into a single primitive shared between both sides of the protocol.

Both changes land together under `mono-00d`. The generic shape lands **as-generic** from day one: `Conductor<Name extends string = string, Role extends 'client' | 'agent' = 'client'>`.

## Motivation

### The current two-call pattern is a paper cut at best and a semantic mismatch at worst

```ts
// Today — two calls, two mental models
const spec = compose(sandbox(...), middleware([...]), agent([...]))
const handle = await spec.start({ serverUrl })  // provision
const acp = await connectAcp(handle.acp)        // transport
await acp.newSession({ ... })
```

The caller has to know:
- That "starting" returns URLs, not a live connection.
- That a second library (`use-acp`, `connectAcp`, or raw `@agentclientprotocol/sdk` wiring) is needed to speak ACP.
- That browser and Node take different paths for the second call.

Every hook-based UI on top of this (see `/Users/gnijor/smithery/flamecast/convert_to_fireline.md` §4) re-introduces a wrapper layer just to re-couple the two.

### "Harness" hides the ACP vocabulary

The Rust SDK names the composed-aggregate-of-components type `Conductor`. The [top-level doctest](https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor.rs#L84-L100) shows exactly the shape:

```rust
let conductor = Conductor::new(
    "my-conductor",
    |cx, conductor_tx, init_req| async move {
        let mut components = Vec::new();
        if needs_auth {
            components.push(spawn_auth_proxy(&cx, &conductor_tx)?);
        }
        components.push(spawn_agent(&cx, &conductor_tx)?);
        Ok((init_req, components))
    },
    None,
);
```

Fireline's `compose(sandbox, middleware, agent)` does the same thing: it builds a list of components. The aggregate should be named `Conductor`, not `Harness`. "Harness" was a reasonable internal term when Fireline was a runner for a single agent process; now that middleware is a first-class proxy chain, the Rust SDK's vocabulary fits exactly.

### ACP's canonical primitive is `connect_to(transport)`

The Rust SDK:

```rust
// agent-client-protocol-conductor/src/conductor.rs#L144-L199
impl<Host: ConductorHostRole> ConnectTo<Host::Counterpart> for ConductorImpl<Host> {
    async fn connect_to(
        self,
        client: impl ConnectTo<Host>,
    ) -> Result<(), agent_client_protocol_core::Error> { /* ... */ }
}

pub async fn run(
    self,
    transport: impl ConnectTo<Host>,
) -> Result<(), agent_client_protocol_core::Error>
```

The TS SDK:

```ts
// @agentclientprotocol/sdk
new ClientSideConnection(toAgent: (conn) => Agent, stream: Stream)
new AgentSideConnection(toClient: (agent) => Client, stream: Stream)
```

Both are transport-agnostic. Both take "the wiring" (a closure that produces the counterpart role) plus "the transport" (a `ConnectTo` / `Stream`). The SDK *already* refuses to take a stance on transport — that's intentional, and correct.

Fireline's `HarnessSpec` today does not — hence this proposal: rename it `ConductorSpec` and expose `.connect_to(transport)` on the runnable `Conductor` layer.

## Proposed API

```ts
// packages/client/src/types.ts

/** Serializable conductor definition — pure data, safe to ship over the wire. */
export interface ConductorSpec<Name extends string = string> {
  readonly kind: 'conductor'
  readonly name: Name
  readonly sandbox: SandboxDefinition
  readonly middleware: MiddlewareChain
  readonly agent: AgentConfig
  readonly stateStream?: string
}

/**
 * Runnable conductor value produced by `compose()`.
 *
 * Matches the shape of Rust `ConductorImpl<Host>` —
 * an aggregate of components (sandbox + middleware + agent) that terminates
 * onto a transport via `.connect_to(transport)`.
 *
 * The `Role` parameter encodes `ConductorHostRole` (client-facing by default;
 * agent-facing when Fireline is composed as a reverse proxy in front of an
 * upstream agent).
 */
export interface Conductor<
  Name extends string = string,
  Role extends 'client' | 'agent' = 'client',
> extends ConductorSpec<Name> {
  readonly role: Role

  /** Return a renamed conductor preserving sandbox/middleware/agent wiring. */
  as<NextName extends string>(name: NextName): Conductor<NextName, Role>

  /** Return a role-cast conductor (rare; used for agent-facing proxy topologies). */
  asRole<NextRole extends 'client' | 'agent'>(role: NextRole): Conductor<Name, NextRole>

  /**
   * Terminate this conductor onto a transport and return a live SDK connection.
   *
   * Mirrors Rust `ConductorImpl<Host>::connect_to(impl ConnectTo<Host::Counterpart>)`.
   *
   * Returns `acp.ClientSideConnection` when Role=client, `acp.AgentSideConnection`
   * when Role=agent — the counterpart role of the conductor itself.
   */
  connect_to(
    transport: ConductorTransport<Role>,
  ): Promise<
    Role extends 'client' ? acp.ClientSideConnection : acp.AgentSideConnection
  >
}

export type ConductorTransport<Role extends 'client' | 'agent' = 'client'> =
  | HostedTransport
  | WebSocketStream
  | StdioTransport
  | CustomStream

export interface HostedTransport {
  readonly kind: 'hosted'
  readonly url: string     // control plane base (POST /v1/sandboxes then WS /acp)
  readonly token?: string
}

export interface WebSocketStream {
  readonly kind: 'websocket'
  readonly ws: WebSocket   // already open; `connect_to` adapts to acp.Stream
}

export interface StdioTransport {
  readonly kind: 'stdio'
  readonly stdin: WritableStream<Uint8Array>
  readonly stdout: ReadableStream<Uint8Array>
}

export interface CustomStream {
  readonly kind: 'stream'
  readonly stream: acp.Stream
}
```

`compose()` is retained as the factory — its return type changes from `Harness<'default'>` to `Conductor<'default', 'client'>`. The `compose()` name reads naturally ("compose a conductor from components") and doesn't fight the rename.

### Usage

```ts
// Hosted (Fireline control plane provisions sandbox, returns WS URL)
const conductor = compose(sandbox({ provider: 'docker' }), middleware([trace(), approve()]), agent(['node', 'agent.mjs']))
const acp = await conductor.connect_to({ kind: 'hosted', url: 'http://localhost:4440' })
await acp.newSession({ cwd, mcpServers: [] })

// Raw browser WebSocket (component is hosted elsewhere, or deployed peer)
const ws = new WebSocket(peerAcpUrl)
await openReady(ws)
const acp = await conductor.connect_to({ kind: 'websocket', ws })

// Node subprocess (editor wires `npx fireline acp-stdio <spec>` via stdio)
const acp = await conductor.connect_to({ kind: 'stdio', stdin: proc.stdin, stdout: proc.stdout })

// Direct Stream (testing, in-memory, or any acp.Stream)
const acp = await conductor.connect_to({ kind: 'stream', stream: memoryStream })
```

One API, four transports, zero caller-side branching on environment.

## Semantic richness captured from Rust `ConductorImpl`

This isn't a cosmetic rename. The Rust conductor type carries real structural semantics we want to preserve in TS:

### 1. The conductor IS an aggregate of components

Rust: `ConductorImpl<Host>` owns the full proxy chain. `connect_to(transport)` terminates the chain onto a transport and runs it. The [conductor doctest](https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor.rs#L84-L100) literally shows an instantiator returning `Vec<Component>`.

TS (proposed): `Conductor<Name, Role>` **is** the composed thing. `compose(sandbox, middleware, agent)` builds the component list. `.connect_to(transport)` terminates it.

The shift from "harness spec → provisioning request → handle → opens a WS → construct a client" (5 concepts) to "conductor → `.connect_to(transport)`" (2 concepts) is the point.

### 2. `ConductorHostRole` — role-directional typing

Rust: `ConductorHostRole` tags the conductor as either `Client`-facing or `Agent`-facing. `ConnectTo<Host::Counterpart>` enforces that a client-facing conductor only accepts client-side transports, and vice versa.

TS (proposed): `Conductor<Name, Role extends 'client' | 'agent'>` captures the same role directionality. Default is `'client'` (the common case — a UI or orchestrator talking downstream to an agent). The `'agent'` variant is reserved for reverse-proxy / agent-facing compositions where Fireline is the thing upstream callers wire into. `connect_to` returns the counterpart: `acp.ClientSideConnection` for a client conductor, `acp.AgentSideConnection` for an agent conductor.

The role parameter lands in Phase 1, not deferred — it's free type-level information and prevents future API breakage.

### 3. `ConnectTo<Host>` — uniform transport trait

Rust: any type implementing `ConnectTo<Host>` can terminate a conductor. `sacp_tokio::Stdio`, axum WS upgrades, tokio TCP streams all implement it.

TS (proposed): `ConductorTransport<Role>` is the tagged-union equivalent of `ConnectTo<Host>`. Closed to a curated set (`hosted`, `websocket`, `stdio`, `stream`) for a small public surface; users who need arbitrary transports supply `{ kind: 'stream', stream }` with any `acp.Stream`.

### 4. Components connect with the same primitive

Reference: [`agent-client-protocol-conductor/src/conductor/mcp_bridge/actor.rs`](https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor/mcp_bridge/actor.rs) — the canonical SDK pattern:

```rust
.connect_with(transport, async move |mcp_connection_to_client| { /* wire */ })
```

The same `connect_with`/`connect_to` primitive that attaches a conductor to its outer transport also attaches *components within the chain* to their inner transports. That symmetry is important — it's what makes proxy chain composition uniform.

In the TS client world today, components of the chain (middleware) are serialized into a `TopologySpec` and instantiated server-side. Once components can run in TS (see Non-goals, interpretation (b)), their inner wiring should use the same `.connect_to(transport)` primitive recursively.

## Proxy-chain alignment (ACP Proxy Chains RFD)

nikomatsakis's [ACP Proxy Chains RFD](https://agentclientprotocol.com/rfds/proxy-chains) says:

> Enable a universal agent extension mechanism via ACP proxies, components that sit between a client and an agent. Proxies can intercept and transform messages, enabling composable architectures where techniques like context injection, tool coordination, and response filtering can be extracted into reusable components.

The RFD's key architectural claim is that a proxy component **implements both `acp.Client` and `acp.Agent`**: it looks like an `Agent` to the client upstream of it, and looks like a `Client` to the agent downstream. That dual role is what makes the chain composable.

Fireline's middleware — `trace()`, `approve()`, `budget()`, `contextInjection()`, `peer()`, `telegram()`, `webhook()` — are *exactly* proxy components in this sense. Today they're expressed declaratively as `TopologyComponentSpec` and materialized server-side. Under this proposal the **public-facing API shape** already matches the RFD: a `Conductor` is an aggregate of proxy components; `.connect_to(transport)` runs the chain.

When TS-native middleware (interpretation (b) in §Non-goals) lands, each middleware will be a TS class implementing:

```ts
interface ProxyComponent extends acp.Agent, acp.Client {
  // acp.Agent methods — called by the upstream client
  newSession(req, opts): Promise<acp.NewSessionResponse>
  prompt(req, opts): Promise<acp.PromptResponse>
  // ...

  // acp.Client methods — called by the downstream agent
  sessionUpdate(notif, opts): Promise<void>
  requestPermission(req, opts): Promise<acp.RequestPermissionResponse>
  // ...
}
```

The chain is: `conductor.connect_to(transport)` wires `client <-> m1 <-> m2 <-> ... <-> agent`, where each `mX` implements both roles. This is the RFD's shape, and it falls out naturally from this proposal — we just have to not paint ourselves into a corner API-wise now.

## Transport taxonomy

| Shape | When | Host process | Transport implementation |
|---|---|---|---|
| `{ kind: 'hosted', url, token? }` | Flamecast-style UIs against a deployed Fireline control plane. | Fireline host provisions subprocess on POST, returns ACP WS. | `connect_to` POSTs, then opens the returned WS and adapts it to `acp.Stream`. |
| `{ kind: 'websocket', ws }` | Browser talking to a peer/deployed agent whose WS URL is already known (e.g., pi-acp always-on). | Remote. | Adapts the open `WebSocket` to `acp.Stream` directly. |
| `{ kind: 'stdio', stdin, stdout }` | Node context spawning a subprocess (editors, CLIs like Zed/Codex/Gemini wrapping `npx fireline acp-stdio`). | Local subprocess. | Wraps stdio streams as a newline-delimited JSON-RPC `acp.Stream`. |
| `{ kind: 'stream', stream }` | Tests, in-memory bridges, future custom transports. | Any. | Passes the `Stream` through unchanged. |

The hosted variant is where the "provision + connect" coupling lives. That's the *only* variant that hits `/v1/sandboxes`. The other three assume the conductor already runs somewhere and the caller supplies a transport to it.

## Migration path

### Phase 1 — add `.connect_to(transport)` alongside `.start()`

- Rename `createHarness` → `createConductor` inside `packages/client/src/sandbox.ts`; keep a `createHarness` alias that returns `Conductor` for one release cycle.
- Rename the `Harness` / `HarnessSpec` / `HarnessConfig` / `HarnessHandle` interfaces to `Conductor` / `ConductorSpec` / `ConductorConfig` / `ConductorHandle` in `packages/client/src/types.ts`; keep `type Harness<Name> = Conductor<Name>` aliases behind `@deprecated` for one release.
- Implement `connect_to` on `Conductor`.
- `.start()` keeps working; internally it delegates to `.connect_to({ kind: 'hosted', url, token })` and drops the WS adapter on the floor so it still returns a `FirelineAgent` handle (or returns the `ClientSideConnection` directly and lets `FirelineAgent` wrap it).
- Add `ConductorTransport<Role>` to `packages/client/src/types.ts`.
- Ship a new helper `adaptWebSocketToStream(ws): acp.Stream` in `@fireline/client`.
- Update `docs/guide/cli.md` "Transport modes" section and the `flamecast/convert_to_fireline.md` §4 skeleton.

### Phase 2 — migrate internal callers

- Replace `compose(...).start({ serverUrl })` + `connectAcp(handle.acp)` call sites inside Fireline examples, demos, and conductor/harness tests with single `connect_to` calls.
- Deprecate `connectAcp` / external `use-acp` coupling in published docs.

### Phase 3 — deprecate `.start()`

- Mark `.start()` `@deprecated` in the public API docs, pointing to `.connect_to({ kind: 'hosted', ... })`.
- In a future major version (post-demo), remove `.start()` and `SandboxHandle` from the public surface; keep them internal to the hosted-transport implementation.

Phases 1 and 2 are additive and ship together. Phase 3 is a follow-up after callers migrate.

## Impact on `flamecast/convert_to_fireline.md` §4

Before (plan-as-written):
```ts
const handle = await compose(...).start({ serverUrl })
const acp = await connectAcp(handle.acp)
// useCreateSession, useTerminateSession, useFirelineAgent all receive `acp`
```

After:
```ts
const conductor = compose(...)
const acp = await conductor.connect_to({ kind: 'hosted', url: serverUrl })
// Same hook surface, one fewer mental hop. `use-acp` dependency goes away.
```

The hook simplification items in the conversion plan's "API gaps" feedback list (items 2, 4, 6) collapse into one: "there should be one way to go from `compose(...)` to a live ACP connection." This proposal is that one way.

## Relationship to `mono-5h6`

`mono-5h6` extracts `wire_conductor(app)` + `serve_stdio(app)` from `routes_acp.rs::handle_upgrade` on the **server side** — decoupling the Rust conductor's wiring from its terminating transport. That work exposes the *same* conductor-shape on Fireline's Rust host that this proposal exposes on Fireline's TypeScript client:

| | Server (Rust) | Client (TypeScript) |
|---|---|---|
| Wiring step | `wire_conductor(app_state)` | `compose(sandbox, middleware, agent)` |
| Terminate on WS | `axum::ws::upgrade → run_on_ws` | `conductor.connect_to({ kind: 'websocket', ws })` |
| Terminate on stdio | `serve_stdio(app) → sacp_tokio::Stdio::new()` | `conductor.connect_to({ kind: 'stdio', stdin, stdout })` |

Both sides of the protocol now speak the same vocabulary. This is the payoff mono-5h6 set up; this proposal cashes it in on the TS side.

## The TS Conductor IS a proxy-mode conductor

This framing is the load-bearing insight of the proposal and deserves its own section.

The Rust SDK has two conductor construction modes:

```rust
// Full mode — conductor owns the component graph end-to-end.
ConductorImpl::new(role, name, instantiator, mcp_bridge_mode)

// Proxy mode — conductor forwards to another conductor over a transport.
impl ConductorImpl<Proxy> {
    pub fn new_proxy(
        name: impl ToString,
        instantiator: impl InstantiateProxies + 'static,
        mcp_bridge_mode: McpBridgeMode,
    ) -> Self { /* ... */ }
}
```

See [`ConductorImpl::new_proxy`](https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor.rs) and the `Proxy` role marker in [`agent-client-protocol-core/src/concepts/proxies.rs`](https://github.com/agentclientprotocol/rust-sdk/blob/main/src/agent-client-protocol-core/src/concepts/proxies.rs#L7).

**A Fireline TS `Conductor` under interpretation (a) below is literally `ConductorImpl::new_proxy` in the Rust taxonomy** — one conductor (the TS one) forwarding to another conductor (the Fireline host running in Rust) over a transport (WS or stdio). That's not "shipping a stub until we can do the real thing"; that is *the* canonical ACP pattern for a client-side conductor that terminates onto a remote component graph.

This changes the framing of the interpretation (a) / (b) split: they're not "stub" and "real" — they're **`new_proxy`** and **`new`**, both first-class construction modes in the Rust SDK today.

## Construction modes

**Mode (a) — `new_proxy` (ships in this proposal)**

The TS conductor owns the *spec* of the component graph (sandbox + middleware + agent), not its execution. `.connect_to(transport)` forwards to a remote Fireline host — in hosted mode, via `/v1/sandboxes` + the WS it returns — and that host instantiates the components in Rust. This is exactly what the Rust `new_proxy` construction does: one conductor forwards to another over a transport.

This interpretation captures 100% of today's Fireline deployments. There is no `start()` + `connectAcp` split any more; there is one call, `conductor.connect_to({ kind: 'hosted', url })`, and the proxy semantics are explicit.

**Mode (b) — `new` with in-process components (future, out of scope)**

For compositions whose middleware is pure TS and whose agent is a direct HTTPS API (Anthropic managed, OpenAI-compatible), the conductor could execute entirely in-process — no remote Fireline host round-trip. Middleware classes implement `acp.Agent & acp.Client` (§Proxy-chain alignment). Durable-streams/persistence stay remote; the proxy chain itself is local TS objects. This is the TS equivalent of Rust `ConductorImpl::new(...)` with a TS-native instantiator.

The API shape here is forward-compatible with mode (b). `.connect_to(transport)` returning an `acp.ClientSideConnection` is exactly the shape an in-process conductor would expose. But mode (b) is not what ships under `mono-00d`.

## Non-goals

- No changes to `routes_acp.rs` beyond what `mono-5h6` already does.
- No changes to the durable-streams substrate or `@fireline/state`.
- No changes to the `@agentclientprotocol/sdk` itself. The SDK is transport-agnostic by design; we build on top.
- No removal of `SandboxHandle` / `Sandbox.provision()` in this round — those become internal implementation details of the hosted transport.
- No mode (b) in this proposal. The API shape stays forward-compatible; the implementation does not ship.

## Resolved decisions

1. **Name: `Conductor`, not `Harness`.** The Rust SDK established the vocabulary; "harness" is Fireline-local jargon that hides the parallel.
2. **Generics land at Phase 1, not deferred.** `Conductor<Name extends string = string, Role extends 'client' | 'agent' = 'client'>`. Free type-level information, prevents API breakage later.
3. **Method name: `connect_to`, not `connectTo`.** Matches the Rust `ConductorImpl::connect_to` method name letter-for-letter. The cross-language symmetry is the point; snake_case on a single public method is cheap surface-wise.

## Open questions

1. **Return type shape for hosted transport** — does `.connect_to({ kind: 'hosted' })` return just `acp.ClientSideConnection`, or a richer `{ acp, state, stop() }` that also exposes the durable-state endpoint today returned in `SandboxHandle.state`? Leaning toward richer; the state endpoint is first-class.
2. **Browser `WebSocket` vs Node `ws`** — the `{ kind: 'websocket', ws }` shape needs to accept both. The WHATWG `WebSocket` interface is standard but Node-land often ships `ws.WebSocket`. A narrow duck-typed interface (`send`, `close`, `addEventListener`, `readyState`) suffices.
3. **Deprecation-aliases window** — how long do `type Harness = Conductor` / `type HarnessSpec = ConductorSpec` aliases live? Proposal: one minor release cycle (demo ships on `Conductor`; aliases drop in the following release). Aliases spare external callers a breaking change while internal callers migrate.

## References

- Rust `ConductorImpl::new` (full mode, doctest showing component aggregation): https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor.rs#L84-L100
- Rust `ConductorImpl::connect_to`: https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor.rs#L144-L199
- Rust `ConductorImpl::new_proxy` (proxy mode — the shape the TS Conductor takes under interpretation (a)): https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor.rs
- ACP core `Proxy` role marker: https://github.com/agentclientprotocol/rust-sdk/blob/main/src/agent-client-protocol-core/src/concepts/proxies.rs#L7
- mcp_bridge actor pattern: https://github.com/agentclientprotocol/rust-sdk/blob/ec9ceae869b240643ce73bca1b9daf7c266c116b/src/agent-client-protocol-conductor/src/conductor/mcp_bridge/actor.rs
- TS SDK transport-agnostic constructor: https://github.com/agentclientprotocol/typescript-sdk/blob/main/src/acp.ts#L531
- ACP Proxy Chains RFD: https://agentclientprotocol.com/rfds/proxy-chains
- ACP transports spec: https://agentclientprotocol.com/protocol/transports
- Fireline TS `HarnessSpec` today: `packages/client/src/sandbox.ts`, `packages/client/src/types.ts#L578`
- Fireline WS terminator today: `crates/fireline-harness/src/routes_acp.rs`
- Server-side decoupling counterpart: bead `mono-5h6`
- Flamecast conversion plan: `/Users/gnijor/smithery/flamecast/convert_to_fireline.md` §4
