# Fireline Browser Harness

This package is a live browser-facing integration harness for Fireline.

It verifies the current end-to-end stack:

- Vite browser client
- ACP over WebSocket via `/acp`
- durable state over `/v1/stream/:name`
- runtime-owned terminal reattach against `fireline-testy-load`

## Run

From the repo root:

```sh
pnpm --filter @fireline/browser-harness dev
```

That starts:

- browser-harness control API on `127.0.0.1:4436`
- Fireline control plane on `127.0.0.1:4440`
- Vite on `http://localhost:5173`
- Fireline runtime on `127.0.0.1:4437` only after you launch an agent from the UI

The harness uses a local-only runtime registry and peer directory under
`packages/browser-harness/.tmp/`.

The control server on `4436` is the browser-facing startup authority and talks
to the Fireline control plane on `4440`. The browser should not expect `/acp`
or `/v1/stream/*` on `4437` to exist until the control plane has created a
runtime and reported `ready`.

## E2E

Run:

```sh
pnpm --filter @fireline/browser-harness test:e2e
```

This boots the harness backend, opens a real browser page with `agent-browser`,
and asks a dedicated `e2e.html` driver page to exercise the actual browser-side
contracts:

- ACP session creation and prompt invocation over `/acp`
- durable state observation over `/v1/stream/:name`
- StreamDB projections updating in response to prompt output

The e2e does not assert on the harness UI layout or component tree.

## What It Exercises

- browse launchable ACP registry agents
- launch a selected agent into a Fireline runtime through the control plane
- open a live ACP connection from the browser
- initialize and create a session
- prompt the terminal agent
- disconnect and `session/load` the same session
- observe durable `STATE-PROTOCOL` rows in parallel

## Notes

- This is a harness, not a product UI.
- It intentionally reflects the current single-attachment runtime model.
- Multi-client shared session behavior is deferred to the ACP bridge work.
