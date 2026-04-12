## @fireline/client

`@fireline/client` is Fireline's control-plane package for composing a harness, provisioning a sandbox, and handing ACP/state endpoints back to the caller. It does not wrap ACP itself; once you have a `SandboxHandle`, you talk to the agent with `@agentclientprotocol/sdk` and observe durable state with `@fireline/state`.

### Quick start

```ts
import { Sandbox, agent, compose, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'

const config = compose(sandbox(), [trace()], agent(['npx', '-y', '@anthropic-ai/claude-code-acp']))
const handle = await new Sandbox({ serverUrl: 'http://127.0.0.1:4440' }).provision(config)
const acp = await connectWithAcpSdk(handle.acp.url)
await acp.initialize()
const response = await acp.connection.prompt({ sessionId, prompt: [{ type: 'text', text: 'hello' }] })
```

`connectWithAcpSdk()` stands in for your `@agentclientprotocol/sdk` transport/client setup. The key point is that Fireline gives you `handle.acp.url`; ACP itself stays a third-party concern.

### Core concepts

- Sandbox = execution environment. In the current client surface this is the `Sandbox` class plus the `sandbox({...})` definition used by `compose(...)`.
- Middleware = serializable ACP pipeline interceptors such as `trace()`, `approve()`, `budget()`, `contextInjection()`, and `peer()`.
- Harness = `compose(sandbox, middleware, agent)`. The return value is a serializable `HarnessConfig`, which is the runnable unit sent to the Fireline host.
- State observation = `@fireline/state`. Fireline returns `handle.state.url`; `@fireline/state` subscribes to that durable stream and materializes live collections.
- ACP sessions = `@agentclientprotocol/sdk`. Fireline returns `handle.acp.url`; the ACP SDK owns session creation, prompting, updates, and reconnects.

### API reference

- `Sandbox` from `@fireline/client`
  `provision(config)` posts a composed harness to the Fireline host and returns a `SandboxHandle`.
  The redesign proposal also discusses `execute()`, but that method is not exported in the landed Phase 1-3 package surface.
- `compose(sandbox, middleware, agent)` from `@fireline/client`
  Returns a `HarnessConfig<'default'>`, which is the serializable harness spec you pass into `Sandbox.provision(...)`.
- Middleware helpers from `@fireline/client/middleware`
  `trace()`, `approve()`, `budget()`, `contextInjection()`, and `peer()` each build one serializable middleware spec.
- `SandboxAdmin` from `@fireline/client/admin`
  `get()`, `list()`, `destroy()`, `status()`, and `healthCheck()` are exported today.
  `findOrCreate()` appears in the redesign vocabulary, but it is not part of the current public API yet.
- Types from `@fireline/client`
  `SandboxConfig`, `SandboxHandle`, `HarnessConfig`, `Middleware`, `SandboxStatus`, `SandboxDescriptor`, `SandboxDefinition`, `AgentConfig`, and `Endpoint`.

### Multi-agent topologies

- `peer(harness1, harness2)`
  Proposed typed topology combinator from the redesign docs. Not exported in the current Phase 1-3 package surface.
- `fanout(harness, { count: N })`
  Proposed typed topology combinator. Not exported in the current Phase 1-3 package surface.
- `pipe(harness1, harness2)`
  Proposed typed topology combinator. Not exported in the current Phase 1-3 package surface.
- `peer()`
  Exported today from `@fireline/client/middleware`. This is middleware-level peer wiring that injects the `peer_mcp` topology component inside a single harness.

### Integration patterns

See [examples/](./examples/) for pointers to runnable client flows. The current examples index links directly to the package's ACP, topology, browser, and hosted-sandbox tests until standalone example programs land.

### Architecture

- [Client API redesign](../../docs/proposals/client-api-redesign.md)
- [Sandbox provider model](../../docs/proposals/sandbox-provider-model.md)
