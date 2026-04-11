# 05: TS Host Primitive

## Objective

Prove that `@fireline/client` can expose the runtime lifecycle surface directly
to TypeScript without inventing a separate control plane.

This slice stays intentionally narrow:

- only `client.host` is implemented
- the only provider is `local`
- runtime discovery comes from the Rust-owned runtime registry
- runtime stop/delete is done by the same host client that spawned the local
  `fireline` process

## What this slice proves

- TypeScript can create a local Fireline runtime and receive a real
  `RuntimeDescriptor`.
- The TS client uses the same runtime record model as the Rust
  `RuntimeHost`.
- `client.host.get` and `client.host.list` are registry-backed, not
  process-handle-backed.
- `client.host.stop` and `client.host.delete` work without a helper API because
  the original host client owns the child process.

## What remains deferred

- remote providers (`docker`, `e2b`, `daytona`)
- a control-plane-independent stop/delete path for non-owned runtimes
- `client.acp` and `client.peer` primitives
- richer runtime status transitions beyond `ready -> stopped`

## Validation

- `pnpm --filter @fireline/client build`
- `pnpm --filter @fireline/client test`
- `packages/client/test/host.test.ts`
  - builds `fireline` + `fireline-testy`
  - creates a local runtime with `provider: auto`
  - verifies provider pinning to `local`
  - verifies `get`, `list`, `stop`, and `delete`
