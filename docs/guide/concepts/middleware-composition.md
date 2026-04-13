# Middleware Composition

Most agent stacks make you choose between two bad options:

- keep behavior in application callbacks that are easy to bypass, hard to observe, and hard to port
- hard-code behavior into one runtime path and rewrite it when you move environments

Fireline takes a different route. You author a **declarative harness spec**, and the host lowers that spec into the actual running middleware pipeline.

That is what `compose(...)` means here. It is not "run these JS hooks around the agent." It is "describe the sandbox, middleware, and agent command as data, then let the host build the real system from that description."

## The Smallest Shape To Remember

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, budget, trace } from '@fireline/client/middleware'

const reviewer = compose(
  sandbox(),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 50_000 }),
  ]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
).as('reviewer')

const handle = await reviewer.start({
  serverUrl: 'http://127.0.0.1:4440',
})
```

Read that in two phases:

1. `compose(...)` creates a serializable spec.
2. `start(...)` sends that spec to a Fireline host, which lowers it into the real running topology.

That split is the heart of middleware composition in Fireline.

## What `compose(...)` Actually Builds

`compose(...)` takes three declarative ingredients:

- `sandbox(...)`
  Where and how the agent should run.
- `middleware([...])`
  The behavior rules Fireline should enforce around the agent.
- `agent([...])`
  The ACP-speaking process to launch.

Before `start()`, this is still just data. No sandbox exists yet. No approval gate is intercepting anything yet. No audit stream is being written yet.

That is intentional. A composed harness is portable because it can be:

- serialized
- named with `.as('...')`
- provisioned on a different host later
- interpreted by the runtime that actually owns the sandbox and state stream

## Why This Is Better Than JS Callbacks

If approvals, budgets, secrets handling, and peer routing lived in arbitrary app callbacks, you would immediately lose four things:

- portability across hosts
- auditability of the authored behavior
- host-side enforcement
- a stable lowering path into non-JS runtimes

Fireline keeps middleware as data so the runtime can enforce it from outside the agent process.

That is the product point, not just an implementation detail. A budget or approval gate is useful precisely because it is infrastructure the agent cannot route around.

## The Lowering Step

When you call `start()`, the client turns the composed harness into a provision request.

Conceptually, this:

```ts
compose(
  sandbox(),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['pi-acp']),
)
```

lowers into a request shape like:

```ts
{
  agentCommand: ['pi-acp'],
  topology: {
    components: [
      { name: 'audit', config: { streamName: 'audit:reviewer' } },
      { name: 'approval_gate', config: { /* approval policy config */ } },
    ],
  },
}
```

That topology object is the bridge between the authored TypeScript spec and the Rust runtime.

## What The Host Does With It

On the TypeScript side, `packages/client/src/sandbox.ts` maps each middleware entry to a topology name plus config.

Examples from current `main`:

- `trace()` -> `audit`
- `approve()` -> `approval_gate`
- `budget()` -> `budget`
- `contextInjection()` / `inject()` -> `context_injection`
- `peer()` -> `peer_mcp`
- `attachTools()` -> `attach_tool`
- `secretsProxy()` -> `secrets_injection`
- `webhook()` -> `webhook_subscriber`
- `autoApprove()` -> `auto_approve`

On the Rust side, `crates/fireline-harness/src/host_topology.rs` registers those names and instantiates the real implementations.

That is the critical boundary:

- TypeScript authors the declarative intent
- Rust interprets and enforces the behavior

The behavior does not live in browser callbacks or user-supplied closures. It lives in host-owned runtime components.

## Order Still Matters

Middleware is declarative, but it is not unordered.

The client lowers the middleware array in order, and that order becomes the topology order the host interprets. So this:

```ts
middleware([
  trace(),
  approve({ scope: 'tool_calls' }),
  budget({ tokens: 50_000 }),
])
```

is meaningfully different from a differently ordered chain.

The practical rule:

- write middleware in the order you want the runtime to reason about the turn
- keep "observe everything" middleware such as `trace()` near the front
- keep policy middleware such as approvals and budgets explicit and easy to read

You are still composing a pipeline. You are just composing it as data instead of as ad hoc interceptor code.

## The JSON-Like Spec Is The Portability Trick

Because the harness is serializable, the same authored spec can be lowered on different hosts and providers.

That is why the same composed shape can move between:

- local development
- Docker-backed runs
- hosted or remote Fireline control planes

without rewriting the middleware logic itself.

The host may provision a different sandbox provider, but the authored rule still means the same thing:

- approval is still an approval gate
- budget is still a budget gate
- secrets still resolve at call time
- peer routing still enables the peer MCP surface

That portability is exactly what you lose if the behavior lives in local callbacks instead of in the harness spec.

## A Concrete Example

The code-review demo is a good example because the behavior contract is easy to read:

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, secretsProxy, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const handle = await compose(
  sandbox({ resources: [localPath(repoPath, '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
    }),
  ]),
  agent(agentCommand),
).start({ serverUrl, name: 'code-review-agent' })
```

That does not mean:

- "the app will maybe notice a dangerous tool call later"

It means:

- mount this repo into the sandbox
- trace ACP traffic into a durable audit stream
- pause dangerous tool calls behind the approval gate
- resolve credentials at call time instead of handing plaintext to the agent

The spec is concise because the host owns the complicated part.

## What This Is Not

It is not:

- React middleware
- Express middleware
- user-supplied JS hooks running inside the agent loop
- a magical one-off compiler pass that only works for one demo

It is a stable authored surface for describing runtime behavior that Fireline knows how to build server-side.

That is why the important abstraction is not "a function that wraps another function." It is "a portable topology description the host can interpret."

## Gotchas

- Do not confuse topology helpers with middleware helpers.
  `peer(...)` / `pipe(...)` at the topology level decide how many harnesses you start; `peer()` inside `middleware([...])` enables peer MCP behavior inside one harness.
- Do not assume middleware behavior lives in TypeScript after `start()`.
  The host is enforcing the real behavior.
- Do not think `compose(...)` provisions anything by itself.
  It only authors the harness value; `start()` performs provisioning.
- Do not treat middleware as arbitrary closures.
  If you cannot serialize it, Fireline cannot lower it portably.

## Read This Next

- [Compose and Start](../compose-and-start.md)
- [Middleware](../middleware.md)
- [Providers](../providers.md)
- [Durable Streams](./durable-streams.md)
- [Code Review Agent example](../../../examples/code-review-agent/README.md)
