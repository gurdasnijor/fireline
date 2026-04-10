# 04: Runtime Provider Lifecycle

## Objective

Prove that Fireline can present a stable runtime record and provider-agnostic
host surface without leaking bootstrap details to callers.

This slice stays intentionally small:

- only the `local` provider is implemented
- `auto` resolves once and is pinned onto the runtime descriptor
- runtime records are persisted to a small file-backed registry
- the existing bootstrap flow remains the source of truth for actually starting
  a runtime

## What this slice proves

- Fireline has a concrete `RuntimeDescriptor` surface.
- Runtime creation is owned by `RuntimeHost`, not ad hoc bootstrap callers.
- Provider choice is resolved once (`auto -> local`) and stored on the
  descriptor.
- Runtime records survive stop and remain queryable until deleted.
- The CLI uses the same runtime host surface as tests and future control-plane
  consumers.

## What agentOS patterns we borrowed

- runtime/bootstrap owns process creation
- ACP attach remains separate from runtime discovery/creation
- provider choice is a bootstrap concern, not an ACP client concern
- runtime records are the control-plane object, not raw process handles

## What remains deferred

- remote providers (`docker`, `e2b`, `daytona`)
- status transitions beyond `starting -> ready -> stopped`
- helper API URL publication once helper routes are real
- TypeScript `client.host` implementation
- durable runtime records backed by a richer store than a local TOML registry

## Validation

- `tests/runtime_provider_lifecycle.rs`
  - creates a runtime with `provider: auto`
  - verifies provider pinning to `local`
  - verifies `get`, `list`, `stop`, and `delete`
- `cargo test -q`
