# @fireline/cli

The `fireline` CLI runs declarative agent specs — `npx fireline agent.ts`
boots a durable-streams server, a Fireline control plane, and provisions
the sandbox defined by the spec's default export. Any ACP client can
then connect to the printed URL. It also ships a `build` subcommand
that emits a hosted Fireline OCI image locally and can scaffold
target-specific deployment files, plus a thin `deploy` wrapper that
hands the image off to target-native tooling.

## Usage

```bash
# The spec file must export a compose(...) value as its default export.
npx fireline run agent.ts

# Override the control-plane and durable-streams ports
npx fireline run agent.ts --port 4440 --streams-port 7474

# Name the state stream for resumability
npx fireline run agent.ts --state-stream my-session

# Override the sandbox provider from the spec
npx fireline run agent.ts --provider docker

# Build the hosted image locally
npx fireline build agent.ts

# Build and scaffold a target descriptor
npx fireline build agent.ts --target fly

# Build, generate a deploy manifest, and hand off to Fly.io
npx fireline deploy agent.ts --to fly

# Pass extra flags straight through to the native deploy tool
npx fireline deploy agent.ts --to k8s -- --namespace fireline
```

## Spec shape

```typescript
// agent.ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
)
```

The file must export the result of `compose(...)` as its default export.
Imperative code that calls `.start()` at module scope is NOT compatible
with `fireline run` — that pattern (used by the examples today) should
be invoked with `npx tsx` directly.

## Known limitations

- The existing examples in `examples/` do not export a default spec; they
  call `.start()` imperatively at module scope. They still run with
  `npx tsx examples/<name>/index.ts` (unchanged).
- `fireline build` shells out to `docker build` against the hosted
  Dockerfile in `docker/` and passes the serialized spec as a build arg.
- `fireline deploy` is a thin wrapper over target-native CLIs:
  `flyctl deploy`, `wrangler deploy`, `docker compose up -d`, or
  `kubectl apply -f <generated>`. It does not talk to a Fireline-owned
  deploy endpoint.
- `fireline push` is still deferred to a later phase.
- The `--repl` flag is still a stub. For now, connect any ACP client to
  the printed ACP URL.

## Binary resolution

The CLI looks for the `fireline` and `fireline-streams` Rust binaries
in this order:

1. `$FIRELINE_BIN` / `$FIRELINE_STREAMS_BIN` (absolute paths)
2. Platform-specific optional npm dependency (`@fireline/cli-darwin-arm64`,
   etc.) — not yet published
3. `target/debug/<name>` or `target/release/<name>` relative to the
   workspace root (dev fallback)

For local dev against this workspace, run
`cargo build --bin fireline --bin fireline-streams` once before invoking
the CLI.

## Exit codes

- `0` — clean shutdown via explicit completion
- `1` — error (binary missing, spec invalid, provisioning failed)
- `130` — shutdown via SIGINT (Ctrl+C)
- `143` — shutdown via SIGTERM
