# Fireline TypeScript Primitives

> Primitive-first design for the TypeScript layer.

## Purpose

This document defines the lowest-level TypeScript primitives that should mirror
Fireline's real capabilities.

This is not the final ergonomic SDK.

The question here is:

- what can Fireline actually do?
- how do we project that to TypeScript with minimal opinion?
- what primitive set is sufficient to drive implementation of the backend?

## Design rules

1. Start from architectural surfaces, not product workflows.
2. Keep bootstrap separate from ACP transport.
3. Keep streams first-class.
4. Keep peer calls host-mediated.
5. Make advanced use possible without forcing everyone through a high-level
   session wrapper.

## Primitive namespaces

```ts
client.stream
client.acp
client.state
client.peer
client.host
client.raw
```

These are the systems layer.

## Bootstrap vs ACP transport

This split is critical.

### Bootstrap / runtime ownership

This layer is responsible for:

- creating or locating a runtime
- returning a `RuntimeDescriptor`
- optionally returning a locally owned ACP transport or process handle

This belongs in `client.host` or a control plane such as Flamecast.

### ACP transport

This layer is responsible for:

- speaking ACP over a provided endpoint or attached transport
- initialize
- create/load session
- prompt
- consume updates

This belongs in `client.acp`.

Important rule:

- `client.acp` should not perform hidden discovery
- `client.host` or Flamecast should provide the endpoint or transport handle

This mirrors the distinction between:

- Fireline's intended model: runtime/bootstrap returns descriptors,
  `client.acp` consumes them
- agentOS's model: `AgentOs.create()` owns the sidecar/runtime and
  `AcpClient` simply attaches to the already-running process

## `client.stream`

Durable streams are first-class.

```ts
type StreamEndpoint = {
  url: string;
  headers?: Record<string, string>;
};

type StreamCursor = string | undefined;

type StreamHandle<T> = {
  replay(from?: StreamCursor): AsyncIterable<T>;
  live(from?: StreamCursor): AsyncIterable<T>;
  close(): Promise<void>;
};
```

Initial entry point:

```ts
client.stream.openState(endpoint): StreamHandle<StateEvent>
```

This primitive underlies:

- dashboards
- state-driven operator views
- sinks
- replay tools
- mesh observers

## `client.acp`

ACP should be visible directly.

```ts
type AcpConnectOptions = {
  url: string;
  headers?: Record<string, string>;
};

type AcpAttachOptions =
  | { process: ManagedProcess; stdoutLines: AsyncIterable<string> }
  | { transport: AcpTransport };

type OpenAcpConnection = {
  connection: ClientSideConnection;
  initialize(req?: { meta?: Record<string, unknown> }): Promise<unknown>;
  updates(): AsyncIterable<unknown>;
  close(): Promise<void>;
};
```

Entry points:

```ts
client.acp.connect(options: AcpConnectOptions): Promise<OpenAcpConnection>
client.acp.attach(options: AcpAttachOptions): Promise<OpenAcpConnection>
```

Current implementation note:

- `client.acp.connect({ url, headers? })` is real for hosted Fireline runtimes
- it is a thin wrapper over the official ACP TypeScript SDK
- the SDK-native `ClientSideConnection` is exposed as `.connection`
- `client.acp.attach(...)` is still deferred
- unsupported inbound client capabilities like file system and terminal methods
  currently fail fast rather than pretending to be implemented

Examples:

```ts
const runtime = await client.host.get(runtimeKey);
const acp = await client.acp.connect({ url: runtime.acpUrl });
```

```ts
const runtime = await client.host.create({ provider: "local", agent: "codex" });
const acp = await client.acp.attach({ transport: runtime.acpTransport });
```

## `client.state`

State is a local materialization over the Fireline state stream.

```ts
type StateHandle<TCollections> = {
  snapshot(): TCollections;
  subscribe(): AsyncIterable<StateChange>;
  close(): Promise<void>;
};
```

Entry point:

```ts
client.state.open({ stateStreamUrl }): Promise<StateHandle<FirelineCollections>>
```

This should be implemented by `@fireline/state`, not by a Rust state server.

## `client.peer`

Peer calls are host-mediated and lineage-aware.

```ts
type PeerDescriptor = {
  nodeId: string;
  acpUrl: string;
  stateStreamUrl: string;
  helperApiBaseUrl?: string;
};

type PeerCall = {
  target: PeerDescriptor;
  prompt: string;
  lineage?: {
    traceId: string;
    parentPromptTurnId: string;
    callerNodeId?: string;
    callerRuntimeId?: string;
  };
};
```

Entry points:

```ts
client.peer.list(): Promise<PeerDescriptor[]>
client.peer.call(spec: PeerCall): Promise<PeerCallResult>
```

This is the primitive behind `prompt_peer` and subagent-like behavior.

## `client.host`

Runtime lifecycle lives here.

```ts
type RuntimeDescriptor = {
  runtimeKey: string;
  runtimeId: string;
  nodeId: string;
  provider: "local";
  providerInstanceId: string;
  status: "starting" | "ready" | "busy" | "idle" | "stale" | "broken" | "stopped";
  acpUrl: string;
  stateStreamUrl: string;
  helperApiBaseUrl?: string;
  createdAtMs: number;
  updatedAtMs: number;
};
```

Operations:

```ts
client.host.create(spec)
client.host.get(runtimeKey)
client.host.list()
client.host.stop(runtimeKey)
client.host.delete(runtimeKey)
```

The important rule is that `client.host` owns runtime creation and discovery;
`client.acp` only speaks ACP over what `client.host` returns.

Current implementation note:

- `create`, `get`, and `list` are real
- local `stop` / `delete` work for runtimes owned by the current host client
- there is not yet a separate control-plane path for stopping an arbitrary
  running runtime discovered only through the registry

## `client.raw`

Escape hatches are intentional.

```ts
client.raw.traceRecord(...)
client.raw.acpConnection(...)
client.raw.transport(...)
```

If the primitive layer is honest, most users will not need this often. It still
needs to exist.

## Compositions

These should emerge naturally from the primitives.

### Sessions

A session wrapper is `client.acp` plus a local state view.

### Webhook forwarding

A webhook is a subscription over trace or state plus an HTTP sink.

### Mesh observation

Mesh observation is N trace subscriptions plus lineage joins.

### Local sidecar attach

A local runtime is `client.host.create({ provider: "local" })` plus
`client.acp.attach(...)`, not a different ACP protocol.

## What this unlocks

This primitive set is enough to drive:

- runtime provider lifecycle
- ACP-native mesh peering
- local state materialization
- `session/load`
- a later ergonomic SDK built on top of these same surfaces
