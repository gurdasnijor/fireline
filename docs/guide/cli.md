# CLI — `npx fireline`

This guide is the quick reference for the current `fireline` CLI surface.
For the higher-level walkthrough, examples, and spec shape, start with
[`packages/fireline/README.md`](../../packages/fireline/README.md).

The currently shipped verbs are:

- `run` — boot Fireline locally and provision a spec
- `build` — build a hosted Fireline OCI image from a spec
- `deploy` — build the image and hand it off to a native platform CLI
- `repl` — open the interactive ACP client for a running Fireline host
- `agents` — install ACP agents from the public registry

Authoritative source:
[`packages/fireline/src/cli.ts`](../../packages/fireline/src/cli.ts)

## Usage

```bash
# Run locally. `run` is optional shorthand.
npx fireline run agent.ts
npx fireline agent.ts

# Boot locally, then attach the interactive REPL immediately.
npx fireline run agent.ts --repl
npx fireline agent.ts --repl

# Attach the REPL to an already-running host.
npx fireline repl
npx fireline repl session-123

# Build the hosted image locally.
npx fireline build agent.ts

# Build and scaffold a target descriptor.
npx fireline build agent.ts --target fly

# Build, scaffold, and invoke the native deploy tool.
npx fireline deploy agent.ts --to fly

# Pass extra args through to the native deploy tool.
npx fireline deploy agent.ts --to k8s -- --namespace fireline

# Install an ACP agent by registry id.
npx fireline agents add pi-acp
```

## `fireline run`

`run` boots `fireline-streams`, boots the local Fireline host, provisions
the spec, then prints the sandbox id, ACP URL, and state stream URL.

Without `--repl`, `run` stays in server mode and prints a follow-up hint:

```bash
To interact: npx fireline agent.ts --repl
```

With `--repl`, `run` boots the host, auto-creates a fresh ACP session,
prints the session id in the ready banner, and then drops straight into
the Ink REPL.

Usage:

```bash
fireline run <file.ts> [flags]
fireline <file.ts> [flags]
```

Flags:

| Flag | Default | Description |
| --- | --- | --- |
| `--port <n>` | `4440` | ACP control-plane port |
| `--repl` | `false` | Attach the interactive REPL after boot |
| `--streams-port <n>` | `7474` | Durable-streams port |
| `--state-stream <s>` | auto | Explicit durable state stream name |
| `--name <s>` | from spec or `default` | Logical agent name |
| `--provider <p>` | from spec | Override `sandbox.provider` |
| `--help` / `-h` | — | Print help |

Examples:

```bash
fireline run docs/demos/assets/agent.ts
fireline run docs/demos/assets/agent.ts --repl
fireline run agent.ts --port 4450 --streams-port 7475
```

## `fireline repl`

`repl` connects an interactive ACP client to a running Fireline host.
The current UI is Ink-based: session header, transcript cards, live tool
status, and an input composer in the terminal.

Usage:

```bash
fireline repl
fireline repl <session-id>
```

Behavior:

- `fireline repl` connects to the host at `$FIRELINE_URL` (default:
  `http://127.0.0.1:4440`) and starts a new ACP session
- `fireline repl <session-id>` attaches to an existing session if the
  host advertises resume or load support
- `Ctrl+C`, `Ctrl+D`, or `/quit` exits the REPL

Helpful CLI guardrails:

- if the argument looks like a spec path such as `agent.ts`, the CLI
  points you at `fireline run agent.ts --repl` instead of treating it as
  a session id
- if no host is listening on the configured port, the CLI points you at
  `fireline run <spec>`

Examples:

```bash
fireline repl
fireline repl session-123
FIRELINE_URL=http://127.0.0.1:4450 fireline repl
```

## `fireline build`

`build` assembles the hosted Fireline OCI image locally. It can also
scaffold target-native config, but it does not invoke the native deploy
tool.

Usage:

```bash
fireline build <file.ts> [flags]
```

Flags:

| Flag | Default | Description |
| --- | --- | --- |
| `--target <platform>` | none | Write one scaffold file for `cloudflare`, `docker`, `docker-compose`, `fly`, or `k8s` |
| `--state-stream <s>` | from spec | Override the baked-in durable state stream name |
| `--name <s>` | from spec | Override the baked-in deployment name |
| `--provider <p>` | from spec | Override the baked-in `sandbox.provider` |
| `--help` / `-h` | — | Print help |

Notes:

- `build` shells out to `docker build`
- scaffold target names are build-time names; for Cloudflare deploys,
  the deploy verb uses `cloudflare-containers`

## `fireline deploy`

`deploy` is a thin wrapper: it runs the hosted image build, generates the
target manifest, then hands off to the native platform CLI. It does not
call a Fireline-owned deploy API.

Usage:

```bash
fireline deploy <file.ts> --to <platform> [flags] [-- <native-flags...>]
```

Flags:

| Flag | Default | Description |
| --- | --- | --- |
| `--to <platform>` | required | Native deploy target: `fly`, `cloudflare-containers`, `docker-compose`, or `k8s` |
| `--state-stream <s>` | from spec | Override the baked-in durable state stream name |
| `--name <s>` | from spec | Override the baked-in deployment name |
| `--provider <p>` | from spec | Override the baked-in `sandbox.provider` |
| `--help` / `-h` | — | Print help |
| `--` | — | Pass all remaining args through to the native target CLI |

Current native CLI mapping:

| `--to` value | Native command |
| --- | --- |
| `fly` | `flyctl deploy` |
| `cloudflare-containers` | `wrangler deploy` |
| `docker-compose` | `docker compose up -d` |
| `k8s` | `kubectl apply -f <generated>` |

Example:

```bash
fireline deploy agent.ts --to fly -- --remote-only
```

## `fireline agents`

`agents` forwards to the companion `fireline-agents` binary. The current
surface is intentionally small:

```bash
fireline agents add <id>
fireline agents --help
```

Current command:

- `add <id>` — install an ACP agent by public registry id

This does not change how `run`, `build`, or `deploy` behave; it is only
the registry install surface.

## Env Vars

| Env var | Meaning |
| --- | --- |
| `FIRELINE_URL` | Override the host URL used by `fireline repl` |
| `FIRELINE_BIN` | Override the path to the `fireline` binary |
| `FIRELINE_STREAMS_BIN` | Override the path to the `fireline-streams` binary |
| `FIRELINE_AGENTS_BIN` | Override the path to the `fireline-agents` binary |

## Binary Resolution

The CLI resolves its backing binaries in this order:

1. `FIRELINE_BIN`, `FIRELINE_STREAMS_BIN`, `FIRELINE_AGENTS_BIN`
2. platform-specific package binaries from `@fireline/cli-<platform>`
3. workspace `target/release/<name>`
4. workspace `target/debug/<name>`

For repo-local development, build the Rust binaries before invoking the
CLI:

```bash
cargo build --release --bin fireline --bin fireline-streams
```

If you use `fireline agents` from this repo checkout, also build:

```bash
cargo build --release --bin fireline-agents
```

## Spec Requirement

The file passed to `run`, `build`, or `deploy` must export the result of
`compose(...)` as its default export.

Imperative files that call `.start()` at module scope are not compatible
with `fireline run`; keep using `npx tsx` directly for those.

## Known Limits

- the examples under `examples/` still use the imperative `.start()`
  pattern
- the REPL is interactive-terminal oriented; line editing/history polish,
  completion, and pipe-first modes are not the focus of the current
  landing
- `deploy` is target-native orchestration only; there is no Fireline-owned
  deploy endpoint
- the CLI still spawns an HTTP control plane; the longer-term plan is an
  embedded in-process conductor with stdio transport
