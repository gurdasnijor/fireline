# Slice 01: Minimal Vertical Slice

## Objective

Get one real Fireline conductor path working end to end:

1. build a subprocess-backed conductor
2. run it over an in-memory duplex transport
3. talk to it using a minimal ACP test agent
4. emit durable trace records
5. prove the whole path in an integration test

## In Scope

- `fireline-conductor`
  - conductor builder
  - durable trace writer
  - duplex transport
- `fireline-testy`
  - minimal ACP test agent over stdio
- integration test
  - prompt succeeds
  - durable trace receives records

## Out of Scope

- WebSocket hosting
- stdio hosting for the main binary
- peer component behavior
- runtime provider lifecycle
- TypeScript packages
- helper APIs

## Acceptance

- `cargo check` passes
- the integration test proves prompt + trace emission
- the repo has a stable substrate to build the next slice on

## Follow-on

After this slice:

1. binary bootstrap + `/acp` route
2. TS trace schema + conformance
3. peer component and ACP-native mesh peering

