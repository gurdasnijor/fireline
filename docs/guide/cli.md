# CLI — `npx fireline`

The `@fireline/cli` package (see [`packages/fireline/`](../../packages/fireline/))
runs declarative agent specs. `npx fireline agent.ts` boots a
durable-streams server, a Fireline control plane, and provisions the
sandbox defined by the spec's default export. Any ACP client can then
connect to the printed URL. The currently shipped verb surface is:

- `run` — boot a spec locally
- `build` — build a hosted image from a spec
- `deploy` — hand the hosted image off to a native platform CLI
- `agents` — install ACP agents from the public registry

Package source:
[`packages/fireline/src/cli.ts`](../../packages/fireline/src/cli.ts),
[`packages/fireline/src/resolve-binary.ts`](../../packages/fireline/src/resolve-binary.ts)

## Usage

```bash
# From this repo: install JS deps once, build the Rust binaries once, then run the spec.
pnpm install
cargo build --release --bin fireline --bin fireline-streams
npx fireline docs/demos/assets/agent.ts

# Default: boot everything, provision the spec, print endpoints, wait for Ctrl+C.
npx fireline run agent.ts

# Shorthand — the `run` subcommand is optional.
npx fireline agent.ts

# Build the hosted Fireline OCI image with the spec embedded as a build arg.
npx fireline build agent.ts

# Build and scaffold a target-native deploy descriptor.
npx fireline build agent.ts --target fly

# Install an ACP agent by registry id.
npx fireline agents add pi-acp

# Override the control-plane and durable-streams ports.
npx fireline run agent.ts --port 4440 --streams-port 7474

# Name the state stream for resumability across restarts.
npx fireline run agent.ts --state-stream my-session

# Override the sandbox provider from the spec at the command line.
npx fireline run agent.ts --provider docker

# Show help.
npx fireline --help
```

## Spec shape

The spec file must export a `compose(...)` Harness as its **default
export**. Imperative files that call `.start()` at module scope are not
compatible with `fireline run` — run those with `npx tsx` directly.

```ts
// agent.ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, secretsProxy, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({
    provider: 'local',
    resources: [localPath('.', '/workspace')],
  }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
    }),
  ]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
)
```

## What the CLI does

### `fireline run`

1. Loads the spec file with `tsx/esm`'s `tsImport`
2. Verifies the default export is a `Harness` (has `.start()`)
3. Resolves the `fireline-streams` and `fireline` Rust binaries (see
   [Binary resolution](#binary-resolution))
4. Reuses `fireline-streams` on the streams port if `GET /healthz`
   already returns `200`; otherwise spawns it
5. Refuses early if the chosen Fireline host port already answers
   `GET /healthz`; otherwise spawns
   `fireline --control-plane --port <port> --durable-streams-url …`
6. Waits for both `/healthz` endpoints to report 200
7. Calls `spec.start({ serverUrl, stateStream, name })` — which goes
   through the normal HTTP provisioning path
8. Prints the sandbox id, ACP URL, and state stream URL
9. Waits for `SIGINT` / `SIGTERM`
10. Tears down in reverse order: destroy sandbox → stop control plane →
    stop durable streams

### `fireline build`

1. Loads the spec file with the same `tsx/esm` path as `run`
2. Serializes the Harness into the hosted build manifest
3. Resolves the hosted image Dockerfile at `docker/fireline-host.Dockerfile`
4. Runs `docker build --build-arg FIRELINE_EMBEDDED_SPEC=...`
5. Tags the image as `fireline-<spec-name>:latest`
6. Optionally writes one scaffold file for `cloudflare`, `fly`, `docker`, or `k8s`

`build` does not call a Fireline-owned deploy API. Deployment remains
target-native and sits in a later CLI phase.

### `fireline agents`

`agents` forwards to the shipped `fireline-agents` companion binary.
Current surface:

```bash
npx fireline agents add <id>
npx fireline agents --help
```

Today the only shipped subcommand is:

1. `add <id>` — install an ACP agent by public registry id, for example `pi-acp`

This surface is registry-install only. It does not change how `run` or
`build` behave.

## Output

```
durable-streams ready at http://127.0.0.1:7474/v1/stream

  ✓ fireline ready

    sandbox:   runtime:59f5ed5a-d624-4379-808b-f0ded7751980
    ACP:       ws://127.0.0.1:54896/acp
    state:     http://127.0.0.1:7474/v1/stream/fireline-state-runtime-…

  Press Ctrl+C to shut down.
```

Connect any ACP client to the printed `ACP:` URL to prompt the agent.

## Run flags

| Flag | Default | Description |
|------|---------|-------------|
| `--port <n>` | `4440` | Control-plane port |
| `--streams-port <n>` | `7474` | Durable-streams port |
| `--state-stream <s>` | auto | Explicit durable state stream name |
| `--name <s>` | from spec | Logical agent name |
| `--provider <p>` | from spec | Override sandbox provider |
| `--repl` | `false` | Stub — prints a message, waits on ACP URL |
| `--help` / `-h` | — | Print help |

## Build flags

| Flag | Default | Description |
|------|---------|-------------|
| `--target <platform>` | none | Scaffold `wrangler.toml`, `fly.toml`, `Dockerfile`, or `k8s.yaml` |
| `--state-stream <s>` | from spec | Override the embedded state stream name |
| `--name <s>` | from spec | Override the embedded deployment name |
| `--provider <p>` | from spec | Override the embedded sandbox provider |
| `--help` / `-h` | — | Print help |

## Env vars

- `FIRELINE_BIN` — absolute path to the `fireline` binary
- `FIRELINE_STREAMS_BIN` — absolute path to `fireline-streams`
- `FIRELINE_AGENTS_BIN` — absolute path to the `fireline-agents` binary

## Binary resolution

The CLI looks for the Rust binaries in this order (esbuild / turbo
pattern):

1. `$FIRELINE_BIN` / `$FIRELINE_STREAMS_BIN` env vars
2. `target/release/<name>` walking up from the CLI's own directory
3. `target/debug/<name>` walking up from the CLI's own directory

If one binary is only present in `target/release` and the other only in
`target/debug`, the CLI uses the mixed pair and logs that mismatch as an
info message. It does not auto-build.

For local dev against this workspace, build once before invoking:

```bash
cargo build --release --bin fireline --bin fireline-streams
```

## Exit codes

- `0` — clean shutdown via explicit completion
- `1` — error (binary missing, spec invalid, provisioning failed)
- `130` — shutdown via SIGINT (Ctrl+C)
- `143` — shutdown via SIGTERM

## Known limitations

- Existing examples in `examples/` do not export a default spec — they
  call `.start()` imperatively at module scope. They still run with
  `npx tsx examples/<name>/index.ts` (unchanged).
- `fireline build` packages the hosted image locally, but `fireline deploy --to <platform>` and `fireline push` are deferred to later phases.
- The `--repl` flag is a stub. Connect any ACP client (pi-acp, use-acp,
  claude-code, a custom client) to the printed ACP URL.
- Platform-specific npm packages are not published yet, so repo-local
  `npx fireline` still depends on a local workspace checkout with
  `pnpm install` plus the Rust binaries above.
- The CLI spawns an HTTP control plane. The longer-term plan is an
  embedded in-process conductor with stdio transport; see
  [docs/proposals/declarative-agent-api-design.md](../proposals/declarative-agent-api-design.md).
