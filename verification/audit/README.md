# Fireline Audit Tooling

This crate hosts the mechanical audits for the ACP canonical identifiers migration described in [docs/proposals/acp-canonical-identifiers-verification.md](../../docs/proposals/acp-canonical-identifiers-verification.md).

## What It Does

It provides two checks:

1. A manifest-driven Rust type audit in `build.rs`.
   - The manifest lists agent-layer struct fields that must use canonical ACP types such as `sacp::schema::SessionId` and `sacp::schema::RequestId`.
   - The build script parses the referenced Rust source files with `syn` and reports mismatches.

2. A forbidden-identifier grep audit in `tests/forbidden_identifiers.rs`.
   - This scans selected agent-layer source paths for synthetic identifier tokens such as `prompt_turn_id`, `trace_id`, and `logical_connection_id`.
   - A file may be exempted only by adding the exact header annotation:
     - `fireline-verify: allow-legacy-agent-identifiers`

`tests/plane_disjointness.rs` is a stub for the follow-on runtime audit described in the verification proposal. It exists now so the crate shape and test target are stable before the StateProjector refactor lands.

## Strict Mode Rollout

The crate ships with `strict-audit` disabled by default.

- Default mode:
  - `build.rs` prints warnings for manifest violations.
  - grep/runtime tests no-op.
  - This keeps `cargo check --workspace` usable while the canonical-identifiers refactor is still in flight.

- Strict mode:
  - `cargo build -p fireline-audit --features strict-audit`
  - `build.rs` upgrades any manifest mismatch to a hard build failure.
  - grep/runtime tests execute instead of skipping.

The intended rollout is to flip CI to `--features strict-audit` once ACP canonical identifiers Phase 1.5 lands and the seeded violations have been fixed.

## Manifest Format

`agent_layer_manifest.toml` declares the fields that must become canonical:

```toml
[[field]]
file = "crates/fireline-session/src/lib.rs"
struct = "SessionRecord"
field = "session_id"
expected_type = "sacp::schema::SessionId"
```

Add entries as more agent-layer structs are canonicalized. The build script treats any missing file, missing struct, missing field, or mismatched type as an audit violation.

## Relationship To The Proposal

This crate is the implementation of:

- `docs/proposals/acp-canonical-identifiers-verification.md §3.1`
- `docs/proposals/acp-canonical-identifiers-verification.md §3.3`

It is intentionally conservative: the audit only checks fields explicitly listed in the manifest, and strict mode remains opt-in until the migration reaches the phase that can satisfy the audit.
