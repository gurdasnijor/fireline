# Product Object Model

> Related:
> - [`vision.md`](./vision.md)
> - [`user-surfaces.md`](./user-surfaces.md)
> - [`../state/session-load.md`](../state/session-load.md)
> - [`../ts/primitives.md`](../ts/primitives.md)

## Purpose

If Fireline is going to feel like a real product and not just a runtime
substrate, it needs a stable vocabulary.

The key objects are below.

## 1. Session

The durable record of a run.

A session should answer:

- what happened
- where it ran
- whether it can be resumed
- what child sessions or peer calls it created
- what artifacts, prompts, and outputs belong to it

Important nuance:

Fireline already has meaningful session foundations today:

- durable session rows
- runtime-side `SessionIndex`
- consumer-side `sessions` collection
- local `session/load` coordination

What is still missing is turning `Session` into a clearer top-level product
surface rather than leaving it as an implementation-level state row.

## 2. Workspace

The files and working context an agent operates against.

This is not the same thing as a runtime.

A workspace may be:

- a local folder
- a repo clone
- a synced snapshot
- a mounted project root

The point is to make "work against my code/data" portable across runtime
placements and repeatable across sessions.

## 3. Capability Profile

The portable bundle of agent-facing capabilities and policy.

This is the answer to requests like:

- "it should have my configured MCPs"
- "it should have my secrets"
- "it should have my skills"
- "it should use this model/policy/tool budget"

A capability profile is distinct from both workspace and runtime:

- runtime says where execution happens
- workspace says what files/context the run sees
- capability profile says what the run is allowed and able to do

## 4. Runtime

The execution substrate.

Examples:

- local process
- Docker container
- Cloudflare container
- VM / microVM
- later, other provider-backed environments

The runtime is the "hands", not the durable source of truth.

## 5. Agent Run

The binding of:

- session
- workspace
- capability profile
- runtime placement

This is the object most users think they are creating when they "run an
agent."

## Systems Layer vs Product Layer

[`../ts/primitives.md`](../ts/primitives.md) is still the right systems-layer
contract.

Those primitives answer:

- how runtimes are created
- how ACP is spoken
- how state is observed
- how topology is described

But most end users will not think in terms of:

- `client.host.create(...)`
- `client.acp.connect(...)`
- `client.state.open(...)`

They will think in terms of:

- "start a run"
- "resume my session"
- "run this on my repo"
- "use my GitHub and Slack tools"
- "move this to the cloud"
- "show me what happened"

That implies two layers.

### Systems API

Low-level, substrate-oriented:

- `client.host`
- `client.acp`
- `client.state`
- `client.topology`

### Product API / Product UX

Higher-level, user-oriented:

- `sessions`
- `workspaces`
- `profiles`
- `runs`
- approvals, audit, replay, intervention, and sharing

The systems layer stays honest.

The product layer is what makes the system adoptable.
