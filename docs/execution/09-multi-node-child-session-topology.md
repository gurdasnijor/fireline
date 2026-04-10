# 09: Multi-Node Child-Session Topology

## Objective

Persist the distributed session graph explicitly when one Fireline runtime
causes another runtime to create child ACP sessions.

The goal is to move from:

- prompt-turn lineage only

to:

- durable parent -> child session edges across nodes

so control planes and reconnect flows can reason about the actual distributed
session topology, not just the causal prompt tree.

## Why this comes after slice 08

Slice 08 makes terminal and session lifetime explicit at the runtime layer.

That gives Fireline a stable place to hang child-session identity:

- session backends now outlive a single ACP attachment
- `SessionRecord` is already durable and materialized
- the next missing piece is how peer-created child sessions are bound back to
  their parent turn and parent session

Without slice 08, child-session edges would still be attached to unstable
per-transport session semantics.

## What this slice should prove

- when runtime A prompts runtime B, Fireline persists the child session created
  on B
- that child session is durably linked to:
  - the parent prompt turn on A
  - the parent session on A
  - the child runtime on B
- a control plane can reconstruct the distributed session graph from durable
  state alone
- a future reconnect flow can discover which remote child session to attach to

## Durable model

At minimum, Fireline needs one explicit edge record.

Recommended shape:

```ts
type ChildSessionEdge = {
  edgeId: string
  traceId?: string
  parentRuntimeId: string
  parentSessionId: string
  parentPromptTurnId: string
  childRuntimeId: string
  childSessionId: string
  createdAt: number
}
```

This should be projected into the durable state stream as its own entity type,
not inferred later from local host state.

## Why a dedicated edge record

`parentPromptTurnId` on `prompt_turn` is enough to reconstruct the causal tree.
It is not enough to answer session-level questions like:

- what child session did this peer call create?
- which remote runtime owns that child session?
- what should a control plane load if the operator wants to inspect or reattach
  to that child session?

Those are session-topology questions, not just prompt-lineage questions.

## Expected implementation shape

### 1. Capture child session creation in `fireline-peer`

When `prompt_peer` creates a remote ACP session, capture:

- `parentSessionId`
- `parentPromptTurnId`
- `childSessionId`
- target runtime identity

### 2. Emit a durable edge record

Project that edge into the Fireline state stream as a first-class
`STATE-PROTOCOL` row.

### 3. Extend `@fireline/state`

Add the new collection and strict fixture coverage so TS consumers can query:

- `sessions`
- `promptTurns`
- `childSessionEdges`

together.

### 4. Add graph reconstruction tests

Start with:

- 2-node parent/child topology

Then extend to:

- 3-node chain or fork topology if the first path is clean

## Acceptance criteria

- runtime A prompts runtime B over ACP
- runtime B creates a real child session
- Fireline emits a durable child-session edge row
- the edge row points at the correct parent session, parent turn, child session,
  and child runtime
- TS state consumers can reconstruct the distributed session graph from the
  state stream alone

## Non-goals

- shared-session / multiplayer fanout
- multiple concurrent ACP attachments to one session
- remote crash recovery beyond what the existing runtime/session model already
  provides
- replacing the ACP SDK session engine
