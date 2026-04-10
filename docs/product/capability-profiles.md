# Capability Profiles

> Related:
> - [`index.md`](./index.md)
> - [`object-model.md`](./object-model.md)
> - [`product-api-surfaces.md`](./product-api-surfaces.md)
> - [`ecosystem-story.md`](./ecosystem-story.md)
> - [`priorities.md`](./priorities.md)
> - [`../programmable-topology-exploration.md`](../programmable-topology-exploration.md)
> - [`../runtime/lightweight-runtime-provider.md`](../runtime/lightweight-runtime-provider.md)
> - [agent.pw](https://github.com/smithery-ai/agent.pw)

## Purpose

A capability profile is Fireline's portable answer to:

- "it should have my configured MCPs"
- "it should have my secrets"
- "it should have my skills"
- "it should have my default policies"
- "it should feel like the same agent environment across runs"

This doc defines what a capability profile is, what it should own, and what it
should deliberately leave out.

## What A Capability Profile Is

A capability profile is the reusable bundle of:

- tool and MCP access
- credential references
- skill and instruction defaults
- policy defaults
- model and runtime defaults that are agent-facing

It is not:

- the workspace
- the runtime
- the session transcript

Those are separate product objects.

## Why It Needs To Exist

Without a profile model, Fireline risks scattering agent-environment concerns
across:

- runtime bootstrap flags
- local config files
- ad hoc topology config
- UI-only state
- provider-specific environment injection

That would make the system hard to reuse and impossible to port cleanly across:

- local vs remote execution
- interactive vs background runs
- browser vs CLI vs product integrations

The profile exists to give Fireline one portable object for "what this run can
do and how it should behave."

## Core Boundaries

### Profile vs workspace

Workspace answers:

- what code or data does the run see?

Profile answers:

- what tools, credentials, instructions, and policies does the run carry?

### Profile vs runtime

Runtime answers:

- where execution happens
- what provider or substrate is used

Profile answers:

- what the agent is allowed and able to do once it is running

### Profile vs session

Session answers:

- what happened during a particular run

Profile answers:

- what defaults and capabilities the run began with

## What Should Belong In A Profile

### 1. MCP and tool definitions

Examples:

- GitHub MCP
- Slack MCP
- docs/search MCP
- internal company service MCPs

The profile should define:

- which MCP endpoints are available
- how they are named
- whether they are enabled by default
- any static config needed to connect them

### 2. Credential references

Profiles should not store raw credentials directly.

They should store references such as:

- secret ids
- vault paths
- `agent.pw` connection paths
- credential scope descriptors

This is where the `agent.pw` story becomes concrete:

- Fireline profile says *which* credential path a capability needs
- `agent.pw` resolves fresh headers or OAuth tokens at call time

### 3. Skills and instruction defaults

Examples:

- coding style defaults
- review behavior
- documentation tone
- product-specific instructions

These should be attachable as profile defaults rather than being hardcoded into
every run or workspace.

### 4. Policy defaults

Examples:

- approval requirements
- tool allow/deny rules
- budget defaults
- network or external-service restrictions

This is where reusable safety and governance settings belong.

### 5. Model or runtime-facing defaults

Examples:

- preferred catalog agent id
- preferred model class
- default topology components
- default placement hints

These should be allowed, but only when they are agent-facing defaults rather
than provider internals.

## What Should Not Belong In A Profile

Do not put these into the profile:

- a local filesystem path
- a git checkout ref that defines the working copy
- a runtime instance id
- a session transcript
- provider-specific low-level lifecycle details

Those belong to workspaces, runtimes, and sessions.

## Product Shape

At the product layer, a profile should feel like a reusable environment preset.

Suggested questions it should answer:

- what MCPs/tools are attached?
- where do credentials come from?
- what policies apply?
- what reusable instructions apply?
- what defaults should runs inherit if they do not override them?

Suggested surface:

```ts
client.profiles.list()
client.profiles.get(profileId)
client.profiles.create(spec)
client.profiles.update(profileId, patch)
client.profiles.clone(profileId, overrides?)
```

## Strawman Profile Shape

This is not final. It is the intended contour.

```ts
type CapabilityProfile = {
  profileId: string
  name: string
  description?: string

  mcpServers: McpBinding[]
  credentialRefs: CredentialRef[]
  skills: SkillRef[]
  instructionLayers: InstructionLayer[]
  policies: ProfilePolicy[]

  defaults?: {
    agentId?: string
    topology?: TopologySpec
    placementMode?: "local" | "remote" | "auto"
  }

  createdAtMs: number
  updatedAtMs: number
}
```

## How Profiles Map To The Existing System

Profiles should compile down into existing technical surfaces:

| Profile concern | Existing or planned substrate |
|---|---|
| MCP definitions | runtime/bootstrap config, later control-plane-backed config |
| instruction layers | conductor `context_injection` and related components |
| audit or policy defaults | conductor topology components |
| credential refs | external vault / `agent.pw` / control-plane auth broker |
| preferred agent defaults | catalog + run start defaults |

This is why the profile should live above the systems layer, not replace it.

## `agent.pw` As The Credential Layer

The clean split is:

- Fireline owns profiles, runs, sessions, approvals, and runtime placement
- `agent.pw` owns encrypted credential storage, OAuth lifecycle, and header
  resolution

That implies:

- profiles reference credential paths, not raw tokens
- conductor-injected MCP or tool bridges resolve credentials at call time
- OAuth refresh and revocation stay outside the runtime

This is a much better story than baking credentials into runtime images or
environment variables for every run.

## Interaction With Approvals

Profiles and approvals are tightly related.

A profile may declare that certain capabilities require:

- approval before use
- out-of-band credential connect
- elevated policy mode

That means profiles should be able to reference approval policy, but they
should not own the lifecycle of approval requests themselves.

Approval requests belong to the run/session layer.

## First-Cut Recommendation

The first version of profiles should stay intentionally narrow.

Include:

- MCP bindings
- credential references
- instruction defaults
- policy defaults
- optional preferred agent id

Defer:

- per-provider resource sizing
- full template inheritance trees
- organization-wide profile distribution semantics
- very rich dynamic policy languages

## Questions The Next Slice Should Answer

1. What is the minimal profile shape needed to start a run?
2. How are MCP bindings represented in a provider-neutral way?
3. How are credential refs expressed so `agent.pw` can resolve them cleanly?
4. Which policy defaults live in the profile vs the run?
5. How does a profile compile into runtime topology and bootstrap config?

## Non-Goals

This doc does not propose:

- a secret-storage system inside Fireline
- replacing `agent.pw`
- putting all run configuration into profiles
- making profiles provider-specific runtime manifests

The goal is simpler:

**a capability profile should be the portable, reusable object that gives a run
its tools, credential references, instructions, and policy defaults.**
