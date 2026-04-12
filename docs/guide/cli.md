# CLI — `npx fireline`

The `@fireline/cli` package (see [`packages/fireline/`](../../packages/fireline/))
runs declarative agent specs. `npx fireline agent.ts` boots a
durable-streams server, a Fireline control plane, and provisions the
sandbox defined by the spec's default export. Any ACP client can then
connect to the printed URL.

Package source:
[`packages/fireline/src/cli.ts`](../../packages/fireline/src/cli.ts),
[`packages/fireline/src/resolve-binary.ts`](../../packages/fireline/src/resolve-binary.ts)

## Usage

```bash
# Default: boot everything, provision the spec, print endpoints, wait for Ctrl+C.
npx fireline run agent.ts

# Shorthand — the `run` subcommand is optional.
npx fireline agent.ts

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

1. Loads the spec file with `tsx/esm`'s `tsImport`
2. Verifies the default export is a `Harness` (has `.start()`)
3. Resolves the `fireline-streams` and `fireline` Rust binaries (see
   [Binary resolution](#binary-resolution))
4. Spawns `fireline-streams` on the streams port
5. Spawns `fireline --control-plane --port <port> --durable-streams-url
   …` on the control-plane port
6. Waits for both `/healthz` endpoints to report 200
7. Calls `spec.start({ serverUrl, stateStream, name })` — which goes
   through the normal HTTP provisioning path
8. Prints the sandbox id, ACP URL, and state stream URL
9. Waits for `SIGINT` / `SIGTERM`
10. Tears down in reverse order: destroy sandbox → stop control plane →
    stop durable streams

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

## Flags

| Flag | Default | Description |
|------|---------|-------------|
| `--port <n>` | `4440` | Control-plane port |
| `--streams-port <n>` | `7474` | Durable-streams port |
| `--state-stream <s>` | auto | Explicit durable state stream name |
| `--name <s>` | from spec | Logical agent name |
| `--provider <p>` | from spec | Override sandbox provider |
| `--repl` | `false` | Stub — prints a message, waits on ACP URL |
| `--help` / `-h` | — | Print help |

## Env vars

- `FIRELINE_BIN` — absolute path to the `fireline` binary
- `FIRELINE_STREAMS_BIN` — absolute path to `fireline-streams`

## Binary resolution

The CLI looks for the Rust binaries in this order (esbuild / turbo
pattern):

1. `$FIRELINE_BIN` / `$FIRELINE_STREAMS_BIN` env vars
2. Platform-specific optional npm dependency
   (`@fireline/cli-darwin-arm64`, etc.) — **not yet published**
3. `target/debug/<name>` or `target/release/<name>` walking up from the
   CLI's own directory (dev fallback)

For local dev against this workspace, build once before invoking:

```bash
cargo build --bin fireline --bin fireline-streams
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
- `fireline deploy` is not implemented yet — only `run`.
- The `--repl` flag is a stub. Connect any ACP client (pi-acp, use-acp,
  claude-code, a custom client) to the printed ACP URL.
- Platform-specific npm packages are not published yet, so `npx
  fireline` only works against a local `cargo build` today.
- The CLI spawns an HTTP control plane. The longer-term plan is an
  embedded in-process conductor with stdio transport; see
  [docs/proposals/declarative-agent-api-design.md](../proposals/declarative-agent-api-design.md).
