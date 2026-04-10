# Fireline

> Open-source runtime substrate for hosting, tracing, and peering ACP-compatible agents.

Fireline is the thing that makes an ACP agent **durable, observable,
and peerable**. It runs the conductor that sits between an ACP client
and an agent subprocess, and produces a durable stream of entity events
that consumers (browser UIs, CLI tools, control planes, other agents)
subscribe to via TypeScript libraries built on `@durable-streams/state`
and TanStack DB.

Pairs with **[Flamecast](https://github.com/flamecast)** — the
open-source control plane for orchestrating agents that run on
Fireline. Fireline is the substrate; Flamecast is the operator-facing
surface.

## Architecture

Start here: [`docs/architecture.md`](./docs/architecture.md) — the
foundational architectural index.

The short version:

- **Rust** is a durable-stream producer. It runs the conductor, traces
  ACP protocol events, projects them into typed entity events, and
  appends to a durable stream. It does not maintain materialized
  state, does not serve queries, does not own a "state API." That
  responsibility lives in TypeScript.
- **TypeScript** is the consumer surface. Schemas live in
  `@fireline/state` (Zod + `createStateSchema`); the client API lives
  in `@fireline/client`. Browser, Node, and CLI consumers subscribe
  to the stream via `createFirelineDB` and observe state via
  `useLiveQuery` (TanStack DB).
- **No "ACP server."** ACP is a protocol over duplex byte streams.
  The conductor is transport-agnostic; the binary mounts transport
  adapters (WebSocket, stdio, in-memory) at axum routes. Same pattern
  as rivet's "HTTP is an adapter layer for actors" model.

## Repo Layout

```
fireline/
├── Cargo.toml                  # Rust workspace root + binary package
├── package.json                # TS workspace root (pnpm)
├── pnpm-workspace.yaml
├── docs/                       # architecture and design docs
│   └── architecture.md
├── crates/                     # Rust library crates
│   ├── fireline-conductor/     # ACP conductor + correlator + transport adapters
│   └── fireline-peer/          # peer call MCP tool surface + registry + transport
├── packages/                   # TypeScript packages
│   ├── state/                  # @fireline/state — schema, createFirelineDB
│   └── client/                 # @fireline/client — programmatic client API
├── src/                        # Fireline binary
│   ├── main.rs                 # CLI
│   ├── lib.rs                  # internal binary modules
│   └── bin/                    # additional binaries (dashboard, agents CLI, etc.)
└── tests/                      # cross-crate Rust integration tests
```

## Status

This repo is in initial scaffolding. The architectural index is the
first deliverable; crates, packages, and binaries will be scaffolded
next, then implementation begins.

## License

TBD. See `LICENSE` (to be added).
