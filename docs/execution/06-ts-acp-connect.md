# 06: TS ACP Connect

## Objective

Prove that `@fireline/client` can connect to a hosted Fireline runtime over
ACP/WebSocket and drive a prompt turn without inventing a Fireline-specific
session layer.

This slice stays intentionally narrow:

- only `client.acp.connect({ url, headers? })` is implemented
- the ACP client is a thin wrapper around the official TypeScript SDK
- the SDK-native `ClientSideConnection` remains the primary handle
- bootstrap remains in `client.host`
- durable observation remains in `client.state`

## What this slice proves

- TypeScript can connect to `runtime.acpUrl` returned by `client.host`.
- The client can `initialize`, `newSession`, `prompt`, and consume
  `session/update` notifications.
- The same prompt turn is visible through `@fireline/state` on
  `runtime.stateStreamUrl`.
- Fireline does not need a custom TS session engine to expose the hosted ACP
  surface.

## What remains deferred

- `client.acp.attach(...)` for locally owned transports
- file system and terminal request handling
- permission brokerage beyond returning `cancelled`
- higher-level session helpers beyond the primitive ACP connection

## Validation

- `pnpm --filter @fireline/client build`
- `pnpm --filter @fireline/client test`
- `packages/client/test/acp.test.ts`
  - creates a local runtime via `client.host.create`
  - connects to `runtime.acpUrl`
  - initializes, starts a session, and prompts `fireline-testy`
  - verifies streamed ACP updates
  - verifies matching durable state rows through `client.state.open`
