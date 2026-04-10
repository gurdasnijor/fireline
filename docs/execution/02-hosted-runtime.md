# Slice 02: Hosted Runtime

## Objective

Turn the proven conductor substrate into a runnable Fireline host process.

That means:

1. serve `/acp` over WebSocket
2. embed `durable-streams-server` in the same process
3. create a durable trace stream for the runtime
4. prove a browser-style ACP client can prompt the hosted runtime and observe trace output

## In Scope

- `fireline-conductor`
  - WebSocket transport adapter
- `fireline` binary
  - bootstrap
  - `/acp` route
  - CLI startup / shutdown
- integration test
  - hosted ACP prompt succeeds
  - durable trace stream contains emitted records

## Out of Scope

- filesystem helper APIs
- webhook forwarding
- runtime provider lifecycle
- real peer/mesh behavior
- TypeScript packages

## Acceptance

- `cargo run --bin fireline -- <agent>` serves a hosted ACP endpoint
- the hosted runtime integration test passes
- the returned bootstrap handle exposes enough runtime information for later host APIs
