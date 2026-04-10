# 13 Phase 0: Runtime Host and Peer Registry Refactor

Status: planned
Type: prerequisite refactor

Related:

- [`./README.md`](./README.md)
- [`../../runtime/control-and-data-plane.md`](../../runtime/control-and-data-plane.md)
- [`../12-programmable-topology-first-mover.md`](../12-programmable-topology-first-mover.md)

## Objective

Extract the runtime/provider boundary and peer-registry boundary without
changing behavior.

This is the prerequisite that makes the later control-plane and Docker work
reviewable instead of entangled.

## Product Pillar

Provider-neutral runtime fabric.

## User Workflow Unlocked

None directly.

This phase is valuable because it unblocks the next two slices without changing
the current local runtime workflow.

## Why This Is Separate

If this refactor is mixed into control-plane or Docker work, review gets noisy
and regressions become harder to isolate.

This phase should be:

- mechanical
- zero-behavior-change
- easy to validate against existing tests

## Scope

### 1. Extract runtime/provider code into `fireline-conductor`

Move the current `RuntimeHost` and provider-backed lifecycle internals out of
the binary crate and into `fireline-conductor`.

Target shape:

- `crates/fireline-conductor/src/runtime/mod.rs`
- `crates/fireline-conductor/src/runtime/manager.rs`
- `crates/fireline-conductor/src/runtime/provider.rs`
- `crates/fireline-conductor/src/runtime/local.rs`

`RuntimeHost` should remain the public lifecycle surface.

### 2. Introduce `RuntimeProvider`

Extract a provider trait behind `RuntimeHost`.

Phase 0 keeps:

- `LocalProvider` only

Phase 0 does not add:

- `DockerProvider`
- `CloudflareProvider`
- any new provider-specific bootstrap logic

### 3. Introduce `PeerRegistry`

Extract a broader runtime/peer registry seam.

Keep the current file-backed implementation as the local adapter.

Expected outcome:

- `LocalPeerDirectory` remains available for local mode
- `PeerComponent` depends on a registry trait instead of a concrete file-backed
  type

### 4. Update imports and composition code

Adjust the runtime binary and existing call sites to use the extracted types.

Likely touch points:

- `src/bootstrap.rs`
- `src/main.rs`
- `src/routes/acp.rs`
- existing tests that import or instantiate runtime-host functionality

## Explicit Non-Goals

This phase does **not** add:

- a control-plane binary
- external durable-streams bootstrap
- endpoint objects on runtime descriptors
- registration or heartbeat
- Docker or mixed-runtime topologies
- TypeScript surface changes

## Acceptance Criteria

- the extracted `RuntimeHost` still supports the current local lifecycle path
- the current local file-backed peer discovery still works through the new
  registry seam
- no wire-level or CLI-level behavior changes are introduced
- existing local runtime and peer behavior remain intact

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`

No new end-to-end topology is required in this phase.

## Handoff Note

This phase is safe to hand to Codex as a pure refactor request.

The prompt should emphasize:

- preserve behavior
- do not add new features
- keep `LocalProvider` as the only provider
- keep the review surface mechanical
