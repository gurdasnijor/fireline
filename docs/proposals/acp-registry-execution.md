# ACP Registry Client Execution Plan

> Status: execution plan
> Date: 2026-04-12
> Scope: ACP registry client in `crates/fireline-tools`, `fireline-agents add <id>` CLI flow, and compose-time `agent(['<id>'])` resolution
> CI-first: per the v2 contention rules in [`docs/status/orchestration-status.md`](../status/orchestration-status.md), GitHub Actions is the sole binding gate; do not use local cargo on the shared worktree

This document is the rollout plan for turning the current ACP registry TODO into a usable install-and-resolve flow.

The end state is small and concrete:

- Fireline can fetch the public ACP registry from `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`
- users can run `fireline-agents add <id>` to install a registry-published ACP agent
- compose specs can use `agent(['pi-acp'])`, and Fireline can resolve that shorthand to a locally installed ACP agent binary

That is the missing link for the `pi-acp` to OpenClaw demo to run against a real ACP agent instead of a placeholder.

## Working Rules

1. Land directly on `main` as short-lived PRs. Do not build a long-lived registry branch.
2. One phase per PR. Each phase must be revertable without partially reverting another phase.
3. CI first only. Do not treat local cargo or ad hoc local install tests as binding on the shared worktree.
4. Preserve the existing explicit-command behavior. `agent(['node', 'agent.js'])` and other already-resolved commands must keep working unchanged throughout the rollout.

## Current State

`crates/fireline-tools/src/agent_catalog.rs` is a TODO stub. Its doc comments already define the intended source of truth:

- registry URL: `https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json`
- registry shape: ACP agent index with `id`, description, install commands, and runtime requirements
- intended role: CLI helper for `fireline-agents add`, not a conductor/runtime concern

The current JS CLI surface in [`packages/fireline/src/cli.ts`](../../packages/fireline/src/cli.ts) only exposes `run` and `help`. There is no install flow yet, so `agent(['pi-acp'])` cannot resolve through a real ACP registry client.

## Compatibility Strategy

Keep the rollout narrow:

- Phase 1 only makes the Rust catalog client real: fetch, deserialize, local cache.
- Phase 2 adds an explicit install command: `fireline-agents add <id>`.
- Phase 3 adds compose fallback only for unresolved single-token agent ids such as `agent(['pi-acp'])`.

This avoids changing the semantics of explicit multi-token commands or path-based agent commands.

## Phase 1: Rust Agent Catalog Client

**Scope**

- Turn [`crates/fireline-tools/src/agent_catalog.rs`](../../crates/fireline-tools/src/agent_catalog.rs) from a stub into a real module.
- Add:
  - registry URL constant
  - HTTP fetch of `registry.json`
  - serde deserialization of the upstream schema as documented in the stub comments
  - local disk cache of the fetched registry
- Keep this phase read-only from the user's perspective. No CLI install behavior yet.

**Gate (CI)**

- Rust workspace build is green for the touched crate set.
- New unit tests cover:
  - successful registry fetch and deserialize
  - local cache read-after-write
  - lookup by known agent id
- A fixture-based test proves the module can load a cached registry without network access.

**Risks**

- Over-normalizing the upstream registry schema instead of consuming it as-is.
- Letting cache location or cache format leak into the public contract.
- Pulling conductor/runtime code into what should remain a CLI-side helper.

**Done when**

- `agent_catalog.rs` is no longer a TODO stub.
- Fireline has a concrete `AgentCatalog` client that can fetch the public registry and reuse a local cache.
- The implementation is explicitly scoped as a CLI/install helper, not as a runtime dependency.

**Rollback**

- Revert the catalog-client PR only. No compose or CLI behavior depends on it yet.

## Phase 2: `fireline-agents add <id>` CLI

**Scope**

- Extend the `packages/fireline` CLI surface so the package exposes `fireline-agents add <id>`.
- The command flow is:
  1. resolve `<id>` against the Phase 1 registry client
  2. surface description/runtime requirements to the user
  3. run the registry-published install command
  4. place the installed binary somewhere Fireline can discover later, such as a managed data dir or another explicit local install location
- Keep this phase explicit. Users opt into installation by running `fireline-agents add <id>` themselves.

**Gate (CI)**

- JS package build/test jobs for `packages/fireline` are green.
- Integration coverage proves:
  - `fireline-agents add <known-id>` resolves a registry entry
  - the install command is invoked through the CLI flow
  - the installed binary lands in the discoverable install location
- Negative-path tests prove:
  - unknown agent id returns a clear error
  - malformed registry entry fails loudly

**Risks**

- Hiding install failures behind generic CLI errors.
- Installing successfully but writing to a location the later compose resolver cannot see.
- Expanding the CLI surface in a way that conflicts with the existing `fireline run` entrypoint instead of complementing it.

**Done when**

- `fireline-agents add <id>` exists on the `packages/fireline` CLI surface.
- A successful install leaves the ACP agent binary in a location the later compose resolver can check deterministically.
- Unknown ids and bad install metadata fail with actionable messages.

**Rollback**

- Revert the CLI-install PR only. The underlying registry client from Phase 1 remains harmless and reusable.

## Phase 3: Compose Integration for `agent(['pi-acp'])`

**Scope**

- Teach compose/start resolution to treat an unresolved single-token agent command like `agent(['pi-acp'])` as ACP registry shorthand.
- Resolution order:
  1. check whether `pi-acp` is already installed in the discoverable location
  2. if not installed, fall through to the `fireline-agents add pi-acp` install flow
  3. re-resolve the installed binary and continue normally
- If the registry is unreachable or the id does not exist, fail with a clear message and stop. Do not silently substitute another agent.
- Keep the fallback narrow: only unresolved single-token ACP agent ids participate. Explicit paths and multi-token commands remain untouched.

**Gate (CI)**

- End-to-end compose tests prove:
  - a spec using `agent(['pi-acp'])` installs/resolves and starts successfully
  - a second run reuses the existing install without re-running the install command
  - registry-unreachable and id-not-found cases fail with clear, deterministic errors
- Existing compose tests for explicit command arrays remain green.

**Risks**

- Accidentally changing the semantics of ordinary single-token binaries that are not ACP registry ids.
- Making compose resolution depend on network reachability even when the agent is already installed.
- Producing a fallback that installs correctly but does not re-enter the normal command-resolution path cleanly.

**Done when**

- `agent(['pi-acp'])` is a valid compose-spec shorthand for a registry-published ACP agent.
- The install fallback only triggers when the agent is not already present.
- The `pi-acp` to OpenClaw demo is unblocked by the same mechanism rather than a demo-only special case.

**Rollback**

- Revert the compose-resolution PR only. Phases 1 and 2 remain useful as explicit registry/install tooling.

## Validation Checklist

- [ ] `docs/proposals/acp-registry-execution.md` exists and stays scoped to the registry client, install CLI, and compose fallback.
- [ ] The public ACP registry URL is the one already documented in `agent_catalog.rs`.
- [ ] The plan keeps the registry schema upstream-owned and does not redesign it locally.
- [ ] The plan keeps caching scoped to fetch + local cache and does not invent a larger cache-control or SLA story.
- [ ] The plan explains how `fireline-agents add <id>` fits into the existing `packages/fireline` CLI surface.
- [ ] The plan explains how Phase 3 unlocks `agent(['pi-acp'])` in compose specs.
- [ ] The plan preserves existing explicit command-array behavior throughout the rollout.

## Architect Review Checklist

- [ ] The execution plan is intentionally small: three phases only.
- [ ] Each phase is independently revertable and small enough for one PR.
- [ ] Each phase has a clear CI gate and does not rely on local cargo runs.
- [ ] Phase 1 is limited to the Rust catalog client plus local cache.
- [ ] Phase 2 adds `fireline-agents add <id>` on the `packages/fireline` CLI surface instead of inventing a separate product boundary.
- [ ] Phase 3 unlocks `agent(['pi-acp'])` with a narrow fallback that does not change explicit command semantics.
- [ ] Error behavior is explicit for registry-unreachable and id-not-found cases.
- [ ] The document does not redesign the upstream ACP registry schema or speculate beyond fetch + local cache.

## References

- [agent_catalog.rs](../../crates/fireline-tools/src/agent_catalog.rs)
- [acp-canonical-identifiers-execution.md](./acp-canonical-identifiers-execution.md)
- [declarative-agent-api-design.md](./declarative-agent-api-design.md)
- [packages/fireline/src/cli.ts](../../packages/fireline/src/cli.ts)
- [proposal-index.md](./proposal-index.md)
