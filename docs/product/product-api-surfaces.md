# Product API Surfaces

> Related:
> - [`index.md`](./index.md)
> - [`vision.md`](./vision.md)
> - [`object-model.md`](./object-model.md)
> - [`user-surfaces.md`](./user-surfaces.md)
> - [`ecosystem-story.md`](./ecosystem-story.md)
> - [`priorities.md`](./priorities.md)
> - [`../ts/primitives.md`](../ts/primitives.md)
> - [`../runtime/control-and-data-plane.md`](../runtime/control-and-data-plane.md)

## Purpose

[`../ts/primitives.md`](../ts/primitives.md) defines the systems-layer API.

This doc defines the layer above it:

- the product-facing objects Fireline should expose
- which surfaces should be user-facing vs internal
- how those surfaces sit on top of the lower-level runtime, ACP, and state
  primitives

This is not a frozen SDK contract.

It is the intended shape of the product layer if Fireline is going to feel like
more than a runtime substrate.

## The Core Separation

Fireline needs two API layers.

### Systems Layer

Low-level, substrate-oriented, already roughly defined today:

- `client.host`
- `client.acp`
- `client.state`
- `client.topology`

These answer:

- how a runtime is created
- how ACP is spoken
- how durable state is observed
- how optional conductor components are composed

### Product Layer

Higher-level, user-oriented, still to be formalized:

- `client.sessions`
- `client.workspaces`
- `client.profiles`
- `client.runs`
- `client.approvals`

These answer:

- what run am I working with?
- what code/data is it working against?
- what tools/secrets/skills/policy does it have?
- where is it currently executing?
- what is waiting on a human or external service?

The systems layer should stay honest.

The product layer should make the system adoptable.

## Product Design Principles

### 1. Session-centric, not runtime-centric

The primary object users care about is not "a process" or "a container."

It is:

- a run
- its transcript
- its artifacts
- its resumability
- its approvals and handoffs

So the product layer should lead with sessions and runs, not runtimes.

### 2. Runtime is an implementation surface

Runtimes still matter, but they belong one layer down.

Most product consumers should ask:

- "where is this run placed?"

not:

- "please give me the ACP URL and state stream URL of provider instance X"

### 3. Workspace and capability should be reusable

Users should not have to re-specify:

- the code/data they want to work against
- the MCPs, skills, secrets, and policies they want attached

on every run.

Those should be reusable objects.

### 4. Approval and waiting must be durable

If a run blocks on:

- a permission request
- an OAuth/credential connect flow
- an operator approval

that wait must survive disconnects and restarts.

### 5. Product APIs should compose from primitives

The product layer should not invent a second hidden execution model.

It should compile down to:

- runtime lifecycle
- ACP transport
- durable state
- conductor topology

## Proposed Top-Level Product Namespaces

### `client.sessions`

Primary durable record surface.

User-facing questions:

- what sessions exist?
- what happened in this session?
- can I reopen it?
- what child sessions did it create?

Suggested responsibilities:

- list sessions
- get one session
- reopen / resume a session
- inspect transcript and metadata
- inspect artifacts and child sessions

Suggested shape:

```ts
client.sessions.list(...)
client.sessions.get(sessionId)
client.sessions.resume(sessionId, options?)
client.sessions.timeline(sessionId)
client.sessions.artifacts(sessionId)
client.sessions.children(sessionId)
```

### `client.workspaces`

Portable file/context surface.

User-facing questions:

- what is this run working against?
- is it a local folder, git ref, or snapshot?
- can I reuse this workspace on another run?

Suggested responsibilities:

- connect a local path
- register a git workspace
- materialize or snapshot a workspace
- inspect mounts and sync policy

Suggested shape:

```ts
client.workspaces.connectLocal({ path })
client.workspaces.connectGit({ repoUrl, ref? })
client.workspaces.get(workspaceId)
client.workspaces.snapshot(workspaceId)
```

### `client.profiles`

Portable capability and policy surface.

User-facing questions:

- which MCPs, skills, and policies does this run have?
- where do credentials come from?
- can I reuse the same environment on another run?

Suggested responsibilities:

- create/get/list profiles
- attach MCP definitions
- attach credential references
- attach skill and policy defaults

Suggested shape:

```ts
client.profiles.list()
client.profiles.get(profileId)
client.profiles.create(spec)
client.profiles.update(profileId, patch)
```

### `client.runs`

Execution entrypoint and lifecycle surface.

This is the highest-level action surface:

- start a run
- place or move execution
- inspect live status
- stop or cancel a run

Suggested responsibilities:

- start from workspace + profile + agent + placement
- create-or-resume a backing session
- expose run placement and status
- connect the run back to runtime and session records

Suggested shape:

```ts
client.runs.start(spec)
client.runs.get(runId)
client.runs.list(...)
client.runs.stop(runId)
client.runs.cancel(runId)
client.runs.move(runId, placement)
```

### `client.approvals`

Durable waiting / human-service surface.

User-facing questions:

- what is blocked right now?
- what needs my approval?
- how do I approve or deny it later?

Suggested responsibilities:

- list pending approval or connect requests
- fetch details
- approve, deny, or expire them
- associate them with sessions and runs

Suggested shape:

```ts
client.approvals.list(...)
client.approvals.get(requestId)
client.approvals.approve(requestId, payload?)
client.approvals.deny(requestId, reason?)
```

## What Stays Internal Or Advanced

Some surfaces should stay mostly internal or expert-oriented even if they exist
publicly.

### `client.host`

Keep as the systems/runtime surface.

Best for:

- infrastructure tooling
- control plane implementation
- advanced local or provider-specific workflows

Not the main surface most product consumers should start with.

### `client.acp`

Keep as the direct ACP transport surface.

Best for:

- harness authors
- protocol tooling
- advanced debugging
- low-level interactive clients

### `client.state`

Keep as the durable observation substrate.

Best for:

- internal control-plane views
- dashboards
- power users
- downstream products doing their own materialization

### `client.topology`

Keep as the advanced extension-composition surface.

Best for:

- framework authors
- platform operators
- advanced runtime templates

Most end users should feel its effects through profiles, templates, and product
features rather than building topology objects manually.

## How The Product Layer Maps To The Systems Layer

The product layer should compile down to the primitives below.

| Product surface | Backing primitives | Notes |
|---|---|---|
| `sessions` | `state`, `acp`, runtime-side session durability | Session is durable first, live ACP second |
| `workspaces` | host/provider bootstrap, helper APIs, later sync mechanisms | Workspace should not be treated as just a runtime path |
| `profiles` | topology, MCP config, credential refs, policy config | This is where `agent.pw` should plug in |
| `runs` | host + acp + state + sessions | Runs are the product entrypoint that bind the rest |
| `approvals` | conductor gates + durable state + control-plane APIs | Waiting must be durable and externally serviceable |

## Environment-Specific API Shape

The product layer will likely need environment-specific packaging even if the
conceptual API stays similar.

### Browser-friendly surface

Should prefer:

- `sessions`
- `runs`
- `approvals`
- read-oriented `workspaces`
- read-oriented `profiles`

It should not assume:

- local process spawn
- direct filesystem access
- Node-only transports

### Node / CLI / server surface

Can expose:

- all product surfaces
- all systems surfaces
- local host spawning
- catalog resolution
- direct control-plane helpers

### Control-plane service surface

Should own:

- canonical mutation APIs for runs, sessions, approvals, workspaces, and
  profiles
- runtime registration and inventory
- auth and external service integration

## Recommended First Product API Shapes

If Fireline wants to make this real incrementally, the first product surfaces
should be:

### First: `runs` and `sessions`

Because they build directly on what already exists:

- durable session state
- runtime lifecycle
- ACP connect
- state materialization

### Second: `approvals`

Because this is one of the strongest differentiated product stories for
long-running agents and out-of-band servicing.

### Third: `profiles`

Because this gives Fireline a strong answer for:

- MCP portability
- credential references
- skills and policy defaults

### Fourth: `workspaces`

Because this is critical for the local-to-remote move story, but it is more
sensitive to provider and sync design choices.

## A Strawman Product Client

This is not a committed API, but it shows the intended feel.

```ts
const workspace = await client.workspaces.connectLocal({
  path: "/Users/me/repo",
})

const profile = await client.profiles.get("coding-default")

const run = await client.runs.start({
  workspace,
  profile,
  placement: { mode: "auto" },
  agent: { source: "catalog", agentId: "codex-acp" },
})

const session = await client.sessions.get(run.sessionId)
const approvals = await client.approvals.list({ runId: run.runId })
```

Or:

```ts
const run = await client.runs.startFromWebhook({
  event: githubPullRequestOpened,
  workspace: { kind: "git", repoUrl, ref },
  profile: "pr-reviewer",
})
```

The important point is not the exact names.

The important point is that the product layer should feel:

- run-centric
- session-aware
- workspace-aware
- profile-aware
- durable

## What This Means For Near-Term Implementation

This doc implies four near-term architectural consequences:

1. `Session` should stop being only a durable row and become a cleaner product
   surface.
2. `Profile` needs a real model before conductor components, MCP config, and
   credential references sprawl in unrelated directions.
3. `Approval` needs durable records and a service API, not just synchronous
   in-process gating.
4. `Workspace` should be modeled explicitly before remote placement and
   migration are productized.

## Non-Goals

This doc does not require:

- hiding the systems layer
- removing direct ACP access
- replacing the existing primitives
- finalizing every method name now

The goal is simpler:

**make it clear what the product layer above Fireline's runtime substrate is
supposed to look like.**
