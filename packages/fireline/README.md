# `@fireline/cli`

`@fireline/cli` is the package behind `npx fireline`. It is the fastest way to
prove a Fireline spec locally, open a live session in the terminal, install ACP
agents from the registry, and turn the same spec into a hosted image.

Use it when the problem is no longer "run one script once" but:

- boot the full local Fireline stack around a spec
- connect to the agent immediately with a REPL
- reconnect to an existing session on a running host
- build or deploy the same spec without rewriting it

## What A Spec Looks Like

`run`, `build`, and `deploy` all expect a TypeScript file whose default export is
the result of `compose(...)`.

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({
    resources: [localPath('.', '/workspace')],
  }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
  ]),
  agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp']),
)
```

Imperative files that call `.start()` at module scope are not CLI specs. Run
those directly with `npx tsx`.

## Fastest Path

For repo-local development, build the binaries once:

```bash
pnpm --filter @fireline/cli build
cargo build --bin fireline --bin fireline-streams --bin fireline-agents

export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
export FIRELINE_AGENTS_BIN="$PWD/target/debug/fireline-agents"
export ANTHROPIC_API_KEY="..."
```

Then boot a spec and drop straight into the REPL:

```bash
npx fireline run agent.ts --repl
```

That single command:

- starts `fireline-streams`
- starts the local Fireline host
- provisions the spec
- creates a fresh ACP session
- opens the Ink REPL against that session

## Command Surface

### `fireline run <file.ts>`

Boots Fireline locally and provisions the spec.

```bash
npx fireline run agent.ts
npx fireline agent.ts
```

Common flags:

- `--port <n>` to change the control-plane port
- `--streams-port <n>` to change the durable-streams port
- `--state-stream <name>` to pin the durable stream name
- `--name <name>` to override the runtime name
- `--provider <provider>` to override `sandbox.provider`

Example:

```bash
npx fireline run agent.ts --port 4450 --streams-port 7475 --state-stream reviewer-demo
```

Without `--repl`, `run` stays in server mode and prints the ACP URL, state
stream URL, and the follow-up hint:

```bash
To interact: npx fireline agent.ts --repl
```

### `fireline run <file.ts> --repl`

Boots the host and immediately chains into the interactive REPL.

```bash
npx fireline run agent.ts --repl
```

What you get on current `main`:

- the ready banner includes the sandbox id, ACP URL, state URL, and session id
- the UI is Ink-based, not a plain line prompt
- the REPL shows live transcript cards and tool activity
- when a state stream is available, pending approvals surface inline and can be
  answered with `y` or `n`

Exit keys:

- `Ctrl+C`
- `Ctrl+D`
- `/quit`

Important: `run --repl` is a one-process convenience path. Exiting that REPL
tears the local host and `fireline-streams` child process down with it.

### `fireline repl`

Attaches the Ink REPL to an already-running Fireline host.

```bash
npx fireline repl
```

By default it connects to `http://127.0.0.1:4440`, or whatever you set in
`FIRELINE_URL`.

This is the right shape when you want the host to stay up in one terminal while
the REPL comes and goes from another.

### `fireline repl <session-id>`

Reconnects the REPL to an existing session when the agent advertises resume or
`loadSession()` support.

```bash
npx fireline repl session-123
FIRELINE_URL=http://127.0.0.1:4450 npx fireline repl session-123
```

Current guardrails:

- if you pass something that looks like `agent.ts`, the CLI points you at
  `fireline run agent.ts --repl`
- if no host is listening, the CLI points you at `fireline run <spec>`

### `fireline build <file.ts>`

Builds the hosted Fireline OCI image locally from the spec.

```bash
npx fireline build agent.ts
```

Common flags:

- `--target <platform>` to scaffold one target config file
- `--state-stream <name>` to override the baked-in stream name
- `--name <name>` to override the baked-in deployment name
- `--provider <provider>` to override the baked-in sandbox provider

Example:

```bash
npx fireline build agent.ts --target fly
```

`build` shells out to `docker build`. It does not deploy anything by itself.

### `fireline deploy <file.ts> --to <platform>`

Builds the same image, generates the target manifest, then hands off to the
native platform CLI.

```bash
npx fireline deploy agent.ts --to fly
```

Current `--to` targets:

- `fly`
- `cloudflare-containers`
- `docker-compose`
- `k8s`

Pass through extra native flags after `--`:

```bash
npx fireline deploy agent.ts --to k8s -- --namespace fireline
```

This is intentionally a thin wrapper. It does not talk to a Fireline-owned
deploy API.

### `fireline agents add <id>`

Installs an ACP agent by public registry id through the companion
`fireline-agents` binary.

```bash
npx fireline agents add pi-acp
```

That command is the current registry install surface. It does not boot the
agent or provision a spec by itself.

## Environment Variables

Some env vars are CLI-specific. Others are the runtime env you typically export
before launching a model-backed or chat-backed spec.

### CLI And Binary Resolution

- `FIRELINE_BIN`
  override the `fireline` binary path
- `FIRELINE_STREAMS_BIN`
  override the `fireline-streams` binary path
- `FIRELINE_AGENTS_BIN`
  override the `fireline-agents` binary path
- `FIRELINE_URL`
  override the host URL used by `fireline repl`

Example:

```bash
export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
export FIRELINE_AGENTS_BIN="$PWD/target/debug/fireline-agents"
export FIRELINE_URL="http://127.0.0.1:4450"
```

### Common Runtime Env You Export Alongside The CLI

- `ANTHROPIC_API_KEY`
  commonly required by model-backed ACP agents or `env:ANTHROPIC_API_KEY` secret refs
- `OTEL_EXPORTER_OTLP_ENDPOINT`
  OTLP endpoint for host-side trace export
- `OTEL_EXPORTER_OTLP_HEADERS`
  OTLP auth header payload, such as Betterstack bearer auth
- `OTEL_SERVICE_NAME`
  service name attached to exported telemetry
- `OTEL_RESOURCE_ATTRIBUTES`
  extra OTLP resource attributes
- `TELEGRAM_BOT_TOKEN`
  bot token for Telegram-backed middleware or bridge flows

Example:

```bash
export ANTHROPIC_API_KEY="..."
export OTEL_EXPORTER_OTLP_ENDPOINT="https://example.ingest"
export OTEL_EXPORTER_OTLP_HEADERS="Authorization=Bearer <token>"
export OTEL_SERVICE_NAME="fireline"
export TELEGRAM_BOT_TOKEN="..."
```

These are not special CLI flags. They are host-owned or agent-owned environment
variables that the launched Fireline runtime and spec can consume.

## Binary Resolution

The CLI resolves `fireline`, `fireline-streams`, and `fireline-agents` in this
order:

1. the explicit env var override
2. the platform-specific optional `@fireline/cli-<platform>` package binary
3. `target/release/<name>` relative to the workspace
4. `target/debug/<name>` relative to the workspace

If a binary cannot be found, the CLI tells you which paths it tried and points
you at the fix: install the package binary, build the Rust binary, or set the
override env var.

## When To Use Which Command

- use `run` when you want a local Fireline host around one spec
- use `run --repl` when you want the fastest live interactive proof
- use `repl` when the host is already running and the REPL should be disposable
- use `build` when you need the hosted image and scaffold only
- use `deploy` when you want the native platform CLI invoked for you
- use `agents add` when the missing piece is an ACP agent binary, not a running host

## Related Docs

- [CLI Guide](../../docs/guide/cli.md)
- [Environment and Config](../../docs/guide/api/env-and-config.md)
- [Quickstart](../../docs/guide/guides/quickstart.md)
- [First Local Agent](../../docs/guide/guides/first-local-agent.md)
- [`@fireline/client`](../client/README.md)
