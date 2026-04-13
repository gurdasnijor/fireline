# Providers and Sandboxes

When a Fireline agent runs, two different things are happening at once:

- the **host** is provisioning, tracing, enforcing middleware, and exposing ACP/state endpoints
- the **sandbox** is the execution environment where the agent process actually lives

If you blur those together, provider choice becomes confusing fast. You start asking the wrong question:

- "is Fireline local or Docker or hosted?"

The better question is:

- "where does this specific agent process run, and what boundary does that sandbox give me?"

That is what sandbox providers decide.

## The Core Idea

A provider is Fireline's answer to "where should this agent run?"

The same authored harness can target different providers because the provider only changes the execution environment. It does not change the higher-level model:

- `compose(...)` still authors the harness
- middleware still lowers into host-owned behavior
- ACP still speaks to the running agent
- the durable stream is still the source of truth

So provider choice is about **execution boundary**, not about rewriting your app.

## What The Sandbox Owns

The sandbox owns the environment the agent runs inside:

- the process itself
- its filesystem view
- mounted resources
- environment variables visible to the agent
- tool execution boundary

That is the "box" the agent experiences.

Depending on provider, that box might be:

- another local `fireline` child process
- a Docker container
- a microVM-backed sandbox
- a remote managed-agent environment

## What The Host Still Owns

The host stays outside that box and still owns the platform behavior:

- provisioning the sandbox
- exposing the ACP endpoint
- exposing the durable state endpoint
- lowering middleware into runtime components
- enforcing approvals, budgets, tracing, and secrets handling
- tracking sandbox descriptors and lifecycle state

This is why switching providers does not mean rewriting your middleware story. The host-side platform behavior is still Fireline's job.

## The Smallest Shape To Remember

These are the provider hints the TypeScript surface accepts on current `main`:

```ts
const local = sandbox()

const explicitLocal = sandbox({
  provider: 'local',
})

const docker = sandbox({
  provider: 'docker',
  image: 'node:22-slim',
})

const anthropic = sandbox({
  provider: 'anthropic',
  model: 'claude-sonnet-4-6',
})

const microsandbox = sandbox({
  provider: 'microsandbox',
})
```

That is the user-facing contract:

- same `sandbox(...)` helper
- same `compose(...)` shape
- different provider hint

What changes is where the agent actually runs.

## The Default Case: Local Subprocess

`sandbox()` with no provider hint means the local subprocess path.

That is the simplest mental model:

- Fireline spawns another local `fireline` process
- mounts resources before launch
- waits for the sandbox to report ready
- returns ACP and state endpoints for the child runtime

This is the best default for:

- local development
- examples
- debugging the middleware and state model quickly

It keeps the loop tight while still exercising the real Fireline control/session/observation split.

## Docker: Same Model, Stronger Packaging Boundary

`sandbox({ provider: 'docker' })` moves the agent into a container-backed runtime.

What you gain:

- image-based packaging
- cleaner dependency isolation
- a more production-shaped environment boundary

What does not change:

- you still get ACP and state endpoints back
- Fireline still owns middleware lowering and durable state
- your harness shape stays the same

So Docker is not a different product mode. It is the same Fireline contract with a different execution substrate.

## Anthropic Managed Provider: Remote Agent Runtime

`sandbox({ provider: 'anthropic' })` points the harness at the Anthropic managed-agents provider surface when the host is built with that provider enabled.

This is the important mental shift:

- the agent environment is no longer a local process or container you own directly
- Fireline still presents the same high-level harness surface

That can be the right fit when you want:

- a hosted execution environment
- a managed-agent backend instead of a local runtime
- the same Fireline composition model around it

Current `main` caveat:

- this provider is feature-gated on the Rust side
- it requires `ANTHROPIC_API_KEY` in the host environment
- it does not yet support Fireline resource mounts

So the concept is real, but host support still depends on how the runtime was built.

## Microsandbox: Typed In The API, Not Fully Wired In The Host

`sandbox({ provider: 'microsandbox' })` exists in the TypeScript provider union because it is an intended provider shape, and there is real microsandbox-backed code in the repo.

But on current `main`, there is an important boundary:

- the microsandbox primitive exists
- the host dispatcher is not yet wiring that provider through as a normal selectable sandbox provider

So this is the honest mental model:

- the provider name is part of the authored surface
- the full host path is still incomplete

That is exactly the kind of distinction a concept doc should make explicit. The provider idea is part of the architecture; the runtime availability is still a host-configuration fact.

## Why Provider Portability Matters

Most teams do not want three different agent definitions for:

- local development
- CI or staging
- production

They want one authored harness and the ability to move it across execution environments as their requirements harden.

That is what the provider model buys you.

The stable part of the system is:

- agent command
- middleware composition
- resource intent
- observation model

The changeable part is:

- where the sandbox boundary lives

That is a much better trade than rebuilding the whole runtime story every time you move from laptop to container to hosted environment.

## What A Provider Choice Really Changes

Provider choice changes things like:

- isolation strength
- packaging model
- mount support
- whether OCI images are involved
- whether the environment is local, containerized, VM-backed, or remote

Provider choice should not change things like:

- whether approvals are durable
- whether prompts are observable
- whether state is replayable
- whether the host can lower middleware into runtime behavior

If switching providers forces you to rethink the whole product model, the abstraction is too weak.

## A Good User Heuristic

Pick the provider based on the boundary you need:

- **local subprocess**
  Fastest feedback loop, easiest development path
- **docker**
  Better packaging and environment isolation
- **anthropic**
  Hosted execution path when that provider is enabled on the host
- **microsandbox**
  VM-style isolation target, but still an honest runtime gap on current `main`

The important point is not memorizing every provider detail. It is understanding that Fireline is separating the authored harness from the execution substrate.

## Gotchas

- Do not confuse provider choice with middleware choice.
  Provider changes where the agent runs; middleware changes how Fireline governs the run.
- Do not assume every typed provider is configured on every host.
  Host build flags and runtime setup still matter.
- Do not assume remote or hosted providers support every local feature.
  Current Anthropic support, for example, does not yet accept Fireline resource mounts.
- Do not treat the sandbox as the whole platform.
  The host still owns ACP/state endpoints, lifecycle, and enforcement.

## Read This Next

- [Providers](../providers.md)
- [Middleware Composition](./middleware-composition.md)
- [Compose and Start](../compose-and-start.md)
- [Crash-Proof Agent example](../../../examples/crash-proof-agent/README.md)
- [docs/proposals/sandbox-provider-model.md](../../proposals/sandbox-provider-model.md)
