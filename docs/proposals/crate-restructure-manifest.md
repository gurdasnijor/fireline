# Crate Restructure Manifest (Option A — 9 primitive-aligned crates)

## Execution status (live — updated 2026-04-11)

| Phase | Description | Status | Commit |
|---|---|---|---|
| 1 | Primitive crate skeletons | ✅ done | `faf8a76` |
| 2 | Register crates in workspace | ✅ done | `3e06b86` |
| 3 | Move tools + resources | ✅ done | `abd5a29` |
| 4 | Move session + sandbox | ✅ done | `283a903`, `a2b8227` |
| 5 | Move harness + orchestration | ✅ done | `5db71e7` |
| 6 | Move runtime + thin root crate | 🔄 in progress | — |
| 7 | Delete dissolved crates (`fireline-conductor`, `fireline-components`) | ⏳ pending | — |
| 8 | Rewrite test imports + `cargo check --workspace` green | ⏳ pending | — |

**Notes on the landed phases:**

- **Phase 4** landed as two commits (`283a903 Move session and sandbox primitives` plus `a2b8227 Drop moved microsandbox source`); the second is the follow-up cleanup of the now-orphaned source files after the sandbox content migrated out of `fireline-components`.
- **Phases 6–8 are serialized.** Phase 6 (moving runtime + thinning the root crate) has to land before Phase 7's delete pass can happen cleanly, and Phase 7 has to land before Phase 8's test-import fixup can be green against a workspace that contains only the new primitive-aligned crates.
- The demo runbook (`../demo-runbook.md`) §"State 0 — restructure not green" documents how to pin to the last restructure-stable commit if Phase 6–8 work leaves `origin/main` mid-move at demo time.

The existing plan below is unchanged — this status block is an overlay to help readers locate where the in-flight work is relative to the committed manifest.

---

Executable manifest for splitting the current Rust workspace into 9
primitive-aligned crates. Each section defines the target crate, which
existing files move into it, and the dependency edges it is allowed to
have. The goal is a clean `cargo check --workspace` with no behavioral
change to the managed-agent test suite.

This is the direct outcome of the TLA Level 1 vocabulary alignment
(commit `7c990d1`). Target crate names line up 1:1 with the Anthropic
primitive taxonomy the client layer already speaks.

## Target layout

```
fireline-session          durable-stream-backed session log + replay
fireline-orchestration    wake(session_id), trigger loop, session index
fireline-harness          ACP adapter, approval gate, effect capture
fireline-sandbox          tool execution container (microsandbox, local, docker)
fireline-resources        mount + fs backend + resource attachment
fireline-tools            registry, capability ref, descriptor projection
fireline-semantics        (exists) pure semantic kernel
fireline-control-plane    (exists) HTTP runtime management surface
fireline-runtime          (new) runtime manager + provider + registry
                           + top-level bin (replaces today's root crate)
```

Plus the top-level binary crate `fireline` stays as the single `[[bin]]`
entry point, but its library responsibilities migrate out into the
9 crates above. `fireline-conductor` and `fireline-components` are
**dissolved** — their contents move into the new primitives.

## Dependency graph (must-honor)

```
fireline-semantics     ← (leaf; no internal deps)
fireline-session       ← semantics
fireline-tools         ← semantics
fireline-resources     ← semantics, session
fireline-sandbox       ← semantics, resources
fireline-harness       ← semantics, session, tools, resources, sandbox
fireline-orchestration ← semantics, session, harness
fireline-runtime       ← all of the above + control-plane
fireline-control-plane ← semantics, session (for runtime descriptor only)
```

Critical edge: **fireline-orchestration MUST NOT depend on
fireline-harness at the primitive-layer trait level**. Orchestration
can invoke harness *concretely* at the binary-assembly layer
(fireline-runtime), but the trait surface stays clean.

## File-by-file move table

### fireline-session

```
src/session_index.rs                  → crates/fireline-session/src/session_index.rs
src/active_turn_index.rs              → crates/fireline-session/src/active_turn_index.rs
src/stream_host.rs                    → crates/fireline-session/src/stream_host.rs
src/runtime_materializer.rs           → crates/fireline-session/src/materializer.rs
crates/fireline-conductor/src/session.rs → crates/fireline-session/src/lib.rs (merge)
```

### fireline-orchestration

```
src/orchestration.rs                  → crates/fireline-orchestration/src/lib.rs
crates/fireline-conductor/src/primitives/orchestration.rs → crates/fireline-orchestration/src/primitive.rs
```

### fireline-harness

```
crates/fireline-conductor/src/primitives/host.rs   → crates/fireline-harness/src/primitive.rs
crates/fireline-components/src/approval.rs         → crates/fireline-harness/src/approval.rs
crates/fireline-components/src/audit.rs            → crates/fireline-harness/src/audit.rs
crates/fireline-components/src/budget.rs           → crates/fireline-harness/src/budget.rs
crates/fireline-components/src/context.rs          → crates/fireline-harness/src/context.rs
crates/fireline-conductor/src/state_projector.rs   → crates/fireline-harness/src/state_projector.rs
crates/fireline-conductor/src/trace.rs             → crates/fireline-harness/src/trace.rs
crates/fireline-conductor/src/topology.rs          → crates/fireline-harness/src/topology.rs
src/routes/acp.rs                                   → crates/fireline-harness/src/routes_acp.rs
```

### fireline-sandbox

```
crates/fireline-conductor/src/primitives/sandbox.rs → crates/fireline-sandbox/src/primitive.rs
crates/fireline-components/src/sandbox/microsandbox.rs → crates/fireline-sandbox/src/microsandbox.rs
crates/fireline-components/src/sandbox/mod.rs       → crates/fireline-sandbox/src/satisfiers.rs
```

### fireline-resources

```
crates/fireline-components/src/fs_backend.rs  → crates/fireline-resources/src/fs_backend.rs
crates/fireline-conductor/src/runtime/mounter.rs → crates/fireline-resources/src/mounter.rs
src/routes/files.rs                            → crates/fireline-resources/src/routes_files.rs
```

### fireline-tools

```
crates/fireline-components/src/tools.rs        → crates/fireline-tools/src/lib.rs
crates/fireline-components/src/attach_tool.rs  → crates/fireline-tools/src/attach.rs
crates/fireline-components/src/smithery.rs     → crates/fireline-tools/src/smithery.rs
crates/fireline-components/src/peer/           → crates/fireline-tools/src/peer/  (whole dir)
```

### fireline-runtime (new)

```
src/runtime_index.rs                → crates/fireline-runtime/src/index.rs
src/runtime_provider.rs             → crates/fireline-runtime/src/provider_trait.rs
src/runtime_host.rs                 → crates/fireline-runtime/src/host_trait.rs
src/runtime_registry.rs             → (deleted; empty shim per stream-as-truth refactor)
src/load_coordinator.rs             → crates/fireline-runtime/src/load_coordinator.rs
src/bootstrap.rs                    → crates/fireline-runtime/src/bootstrap.rs
src/agent_catalog.rs                → crates/fireline-runtime/src/agent_catalog.rs
src/child_session_edge.rs           → crates/fireline-runtime/src/child_session_edge.rs
src/control_plane_client.rs         → crates/fireline-runtime/src/control_plane_client.rs
src/control_plane_peer_registry.rs  → crates/fireline-runtime/src/control_plane_peer_registry.rs
src/connections.rs                  → crates/fireline-runtime/src/connections.rs
src/topology.rs                     → crates/fireline-runtime/src/topology.rs
src/error_codes.rs                  → crates/fireline-runtime/src/error_codes.rs
src/lib.rs                          → crates/fireline-runtime/src/lib.rs  (pruned to re-exports)
crates/fireline-conductor/src/lib.rs         → merge into crates/fireline-runtime/src/lib.rs
crates/fireline-conductor/src/build.rs       → crates/fireline-runtime/src/build.rs
crates/fireline-conductor/src/shared_terminal.rs → crates/fireline-runtime/src/shared_terminal.rs
crates/fireline-conductor/src/runtime/       → crates/fireline-runtime/src/providers/ (whole dir)
crates/fireline-conductor/src/transports/    → crates/fireline-runtime/src/transports/ (whole dir)
```

### fireline-semantics (exists — unchanged)

```
(no moves)
```

### fireline-control-plane (exists — unchanged body)

```
(no file moves; only dependency updates)
```

### Top-level `fireline` crate (binary-only after restructure)

```
src/main.rs                  → src/main.rs                  (stays; thins to runtime::run())
src/routes/mod.rs            → deleted (routes moved)
src/bin/dashboard.rs         → src/bin/dashboard.rs         (stays; updates imports)
src/bin/agents.rs            → src/bin/agents.rs            (stays; updates imports)
src/bin/testy*.rs            → src/bin/testy*.rs            (stays; updates imports)
```

After the restructure the root `Cargo.toml` becomes a pure
**workspace + binary** crate. All library code ships from the 9
primitive crates.

## Tests

Integration tests under `tests/` currently live at the workspace root.
They have two options:

1. **Stay at workspace root** — continue importing from `fireline` top
   crate which re-exports from the primitive crates. Simplest for this
   pass. **Use this.**
2. Move each `tests/managed_agent_*.rs` into the relevant primitive
   crate's `tests/` dir — cleaner long term, but too much scope for the
   restructure PR.

So: `tests/*.rs` stay put. Only the imports adjust (likely just
`use fireline::*` → `use fireline_session::*` etc.) where the test
reaches into a specific primitive.

## Cargo.toml workspace-members (target)

```toml
[workspace]
members = [
    ".",
    "crates/fireline-semantics",
    "crates/fireline-session",
    "crates/fireline-orchestration",
    "crates/fireline-harness",
    "crates/fireline-sandbox",
    "crates/fireline-resources",
    "crates/fireline-tools",
    "crates/fireline-runtime",
    "crates/fireline-control-plane",
    "verification/stateright",
]
```

`fireline-conductor` and `fireline-components` are **removed** from
workspace-members after their contents migrate out.

## Execution order (to minimize break-midway)

1. Create the 9 crate skeletons (`Cargo.toml` + empty `lib.rs`).
2. Register them in `[workspace].members`, keep `fireline-conductor` /
   `fireline-components` still registered during the move.
3. Move leaf crates first: **semantics** (no-op), **tools**, **resources**.
4. Move **session**, then **sandbox**.
5. Move **harness**, then **orchestration**.
6. Move **runtime** (biggest; absorbs conductor glue).
7. Prune `fireline-conductor` and `fireline-components` to empty stubs,
   then delete their directories and remove from workspace members.
8. Fix up `tests/*.rs` imports.
9. `cargo check --workspace` must be green.
10. `cargo test --workspace` optional at this stage but recommended.

## Non-goals for this PR

- **No semantic changes.** This is a pure move. Trait surfaces stay
  identical; only the module path shifts.
- **No combinator-wire changes.** Option C (Rust combinator serde) is a
  separate follow-up that rebases onto the restructured tree.
- **No test migration.** Tests stay at workspace root.
- **No TLA changes.** Level 2 Host/Sandbox split is a parallel track.

## Done when

- `cargo check --workspace` green
- `cargo test --workspace --no-run` green (compile only)
- `cargo test -p fireline-semantics` passes (leaf regression check)
- `cargo test -p fireline-verification` passes (model regression check)
- `git diff main --stat` shows the move as rename-heavy (files renamed,
  not rewritten)
- One commit per phase in §"Execution order" (9–10 commits) so any
  one phase can be reverted cleanly if the cargo graph refuses to
  resolve.
