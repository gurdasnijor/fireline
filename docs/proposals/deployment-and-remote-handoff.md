# Deployment and Remote Handoff

> Status: design rewrite
>
> This version replaces the older flag-heavy deployment proposal with a DX-first design.

## TL;DR

Fireline should have one primary authoring surface: `compose()`. The CLI is a thin runner around that surface.

If a deployment concern has no representation in the agent spec or a companion config file, it should not become a first-class CLI flag. CLI flags are for runtime overrides like `--port`, `--verbose`, and local debugging. They are not where users should encode provider choice, peer topology, or "always-on" behavior.

The design in this doc is:

- `npx fireline agent.ts` stays the local path
- `npx fireline deploy agent.ts --target production` selects a named target from `fireline.config.ts`
- `alwaysOn` is target policy, not a CLI switch
- `peer()` stays in the spec, not on the command line
- provider defaults stay in `sandbox(...)`; environment-specific overrides live in config, not CLI flags

Current reality:

- `packages/fireline/src/cli.ts` only implements local `run`; there is no `deploy` command yet.
- `packages/client/src/sandbox.ts` still expects `start({ serverUrl })`; it has not moved to the no-arg local / `remote` model yet.
- `packages/client/src/types.ts` already has the right raw ingredients for the spec surface: `sandbox.provider`, `resources`, `fsBackend`, labels, and peer middleware config.
- `packages/client/src/agent.ts` and `packages/client/src/db.ts` already expose the imperative runtime surface that apps need after deployment: `FirelineAgent.connect()`, `.resolvePermission()`, `.stop()`, and `fireline.db()`.

## 1. DX Principle

The design rule is simple:

> The `compose()` API is the product surface. The CLI is a thin runner. If a deployment concern has no representation in `compose()` or a companion config file, it should not become a CLI flag.

This implies a strict split:

- The spec file describes portable agent behavior.
- The config file describes environment-specific deployment targets.
- The CLI chooses which spec to run and which target to use.

This keeps the API coherent:

- the agent file stays portable between laptop, CI, and cloud
- target selection is reusable across multiple agent files
- "production" means one named thing, not a shell alias the team half-remembers
- deployment commands stay readable instead of becoming a second configuration language

It also gives Fireline a better product story than the current direction in `docs/gaps-declarative-agent-api.md` and `docs/proposals/declarative-agent-api-design.md`, which still assume too many deploy-time flags.

## 2. Deployment Target Declaration

The right primitive is a project-level `fireline.config.ts`.

Why this beats the alternatives:

- Better than embedding deployment config in `agent.ts`: the agent file should remain a portable runtime spec, not a cloud-environment manifest.
- Better than raw CLI flags: named targets scale to multiple agents, multiple environments, and team use without command drift.
- Better than hidden shell env conventions: the config file is inspectable, reviewable, and can be validated.

### Proposed shape

```ts
// fireline.config.ts
export default {
  defaultTarget: 'staging',
  targets: {
    staging: {
      host: 'https://agents-staging.example.com',
      auth: { tokenFromEnv: 'FIRELINE_TOKEN' },
      sandbox: {
        provider: 'docker',
        image: 'ghcr.io/acme/pi-acp:staging',
      },
      lifecycle: {
        alwaysOn: false,
      },
      state: {
        namespace: 'acme-staging',
      },
    },
    production: {
      host: 'https://agents.example.com',
      auth: { tokenFromEnv: 'FIRELINE_TOKEN' },
      sandbox: {
        provider: 'anthropic',
        model: 'claude-sonnet-4-20250514',
      },
      lifecycle: {
        alwaysOn: true,
      },
      state: {
        namespace: 'acme-prod',
      },
    },
  },
}
```

The important property is not the exact field names. It is where the information lives:

- remote host URL
- auth source
- environment-specific sandbox override
- always-on policy
- stream namespace or tenancy defaults

All of that is environment configuration. None of it belongs in `agent.ts`, and none of it should require its own CLI switch.

### Why not export deployment metadata from `agent.ts`?

That couples one portable agent definition to one deployment environment. It sounds convenient until one file needs `staging`, `production`, `preview`, or customer-specific targets. Then the spec file becomes a config file in disguise.

The spec should answer:

- what is this agent?
- what middleware does it run?
- what peers does it expect?
- what resources does it mount?

The config should answer:

- where does this go?
- how is it authenticated?
- should this target keep it warm?
- what provider override does this environment need?

## 3. Command Model

### Local run

Local run stays simple:

```bash
npx fireline agent.ts
```

This should remain the default learning path. It aligns with the direction already described in `docs/gaps-declarative-agent-api.md` and `docs/proposals/platform-sdk-api-design.md`: local Fireline should feel like running a script, not assembling infrastructure.

Useful local-only overrides are still fine:

```bash
npx fireline agent.ts --port 4440 --verbose
npx fireline agent.ts --provider docker
```

`--provider` is acceptable here only as a local override for testing. It should not be the main deployment story.

### Deploy

The deploy command should be:

```bash
npx fireline deploy agent.ts --target production
```

The semantics are:

1. Load the default export from `agent.ts`.
2. Load `fireline.config.ts`.
3. Resolve the `production` target.
4. Merge the spec with the target's deployment overrides.
5. Send the result to the remote Fireline host declared by that target.

The key point is that `--target` is not configuration. It is a lookup key.

That means `--target` is acceptable without violating the principle. It does not create a second configuration surface. It selects one.

### If there is only one target

If `fireline.config.ts` has `defaultTarget`, then this should work:

```bash
npx fireline deploy agent.ts
```

That keeps the happy path short without forcing people to invent shell aliases.

## 4. What Lives in the Spec vs the Config vs the CLI

### Belongs in `agent.ts`

These are properties of the agent itself:

- `agent([...])`
- `middleware([...])`
- `peer({ peers: [...] })`
- `trace()`, `approve()`, `budget()`, `secretsProxy()`
- mounted resources
- default sandbox provider when it is part of the agent's identity

Example:

```ts
export default compose(
  sandbox({ provider: 'anthropic' }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    peer({ peers: ['reviewer'] }),
  ]),
  agent(['pi-acp']),
)
```

This is portable behavior and should move with the file.

### Belongs in `fireline.config.ts`

These are environment concerns:

- remote Fireline host URL
- auth token source
- default stream namespace / tenant
- deployment labels and placement defaults
- environment-specific provider override
- always-on lifecycle policy

This is where "dev uses local Docker, prod uses Anthropic" belongs if it is an environment difference rather than an agent identity choice.

### Belongs on the CLI

Only runtime overrides and diagnostics:

- `--target`
- `--port`
- `--verbose`
- `--dry-run`
- `--provider` for local test override only

Not acceptable as primary deploy flags:

- `--always-on`
- `--peer`
- `--provider` for real deployments
- `--durable-streams-url`
- `--model`

If a user needs to type those every deploy, the design has already failed.

## 5. What `alwaysOn` Means

`alwaysOn` is not "pass a flag and hope the process stays up." It is a host-side deployment policy.

Concretely, a target with `lifecycle.alwaysOn = true` means:

- the remote Fireline host persists the desired deployment
- the host keeps a sandbox for that deployment running, or recreates it after failure
- the agent is resumed against the same durable state stream when possible
- operators interact with the same logical deployment even if the backing sandbox is replaced

This is exactly why `alwaysOn` belongs in config or spec, not in the CLI. It is durable policy, not an invocation preference.

It also aligns with the wake semantics already formalized in `verification/spec/managed_agents.tla` and enabled in `verification/spec/ManagedAgents.cfg`:

- `WakeOnReadyIsNoop`
- `WakeOnStoppedChangesRuntimeId`
- `WakeOnStoppedPreservesSessionBinding`
- `ConcurrentWakeSingleWinner`
- `SessionDurableAcrossRuntimeDeath`

Those names matter because they tell us what "always on" should actually mean in Fireline:

- a healthy deployment should not churn on every wake
- a crashed runtime can be replaced without losing the session binding
- the logical deployment survives runtime replacement
- concurrent recovery attempts converge

That is the right semantic base for an always-on deployment model.

## 6. Deployment Topology

This part of the previous proposal stays valid. The topology is still hub-and-spoke around durable streams:

```text
┌───────────────────────────────────────────────────────┐
│ Durable Streams Service                              │
│ - session/state truth                                │
│ - cross-host discovery substrate                     │
│ - survives host or sandbox death                     │
└───────────────────────────────────────────────────────┘
                     ▲                  ▲
                     │                  │
          ┌──────────┴──────────┐  ┌────┴──────────────┐
          │ Fireline Host       │  │ Fireline Host     │
          │ local / laptop      │  │ remote / cloud    │
          │ - run agent.ts      │  │ - deploy target   │
          │ - local provider    │  │ - always-on policy│
          └──────────┬──────────┘  └────┬──────────────┘
                     │                  │
                     ▼                  ▼
               local sandbox      remote sandbox
```

The deployment command does not replace this topology. It chooses where in this topology a given spec should live.

The important architectural point remains:

- durable streams is the shared truth plane
- hosts are replaceable
- sandboxes are replaceable
- deployment targets decide placement and policy, not state ownership

## 7. Remote Handoff Story

The handoff story also stays, but it should be framed in DX terms.

The north-star user experience is:

1. Run locally:

   ```bash
   npx fireline agent.ts
   ```

2. Validate the middleware, resources, and behavior.

3. Promote the same file:

   ```bash
   npx fireline deploy agent.ts --target production
   ```

4. The session can continue on the remote target because the durable stream is still the truth.

What makes this credible is not the CLI. It is the runtime model:

- `packages/client/src/agent.ts` already treats the live deployment as a `FirelineAgent`
- `packages/client/src/db.ts` already treats state observation as a stream-backed database
- `packages/client/src/sandbox.ts` already serializes the harness spec into a provision request

The remaining work is mostly packaging and policy:

- target resolution
- deploy command
- host-side deployment records
- local-to-remote resource and secret handoff

Important honesty point: the handoff story is not complete today.

What exists:

- stream-backed state
- wake semantics in the TLA model
- cross-host discovery as a backend concept
- `FirelineAgent` and `fireline.db()` on the client side

What does not exist yet:

- `fireline.config.ts`
- `fireline deploy`
- a finished `start()` API that defaults to local and uses `remote` instead of `serverUrl`
- polished resource and secret promotion UX

So the right claim is:

> Fireline already has the state model that makes remote handoff believable. It does not yet have the DX layer that makes remote handoff pleasant.

That is exactly what this proposal is trying to fix.

## 8. Migration from the Current Single-Host Model

Current state:

- `packages/fireline/src/cli.ts` boots a local streams process and a local host, then calls `spec.start({ serverUrl })`
- `packages/client/src/sandbox.ts` still models `start()` as a remote control-plane call
- the current design docs still assume too many deploy-time flags

Migration plan:

1. Keep `npx fireline agent.ts` working as the local path.
2. Add `fireline.config.ts` target discovery.
3. Add `fireline deploy <file> --target <name>`.
4. Move client `start()` toward the API already described in `docs/proposals/platform-sdk-api-design.md`:
   - `start()` for local
   - `start({ remote })` for remote
5. Introduce host-side deployment records and lifecycle reconciliation for `alwaysOn`.
6. Move deploy-time environment differences out of CLI flags and into target config.

Compatibility rule during migration:

- old local debugging flags can remain temporarily
- new deploy features should not introduce fresh primary flags for provider, peering, or always-on behavior

## Recommendation

Use option (a): `fireline.config.ts` with named targets.

Allow option (c) only in the narrow sense that `--target` is a lookup key into that config.

Reject option (b): do not put deployment targets inside `agent.ts`.

That gives Fireline the cleanest long-term story:

- the spec file defines the agent
- the config file defines where it goes
- the CLI only runs the plan

Anything else turns the CLI into a second API, and Fireline does not need a second API.
