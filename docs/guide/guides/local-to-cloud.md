# Local To Cloud

You should not need one authoring model for your laptop and a different one for deployment.

On current `main`, the Fireline portability story is:

- write one `compose(...)` spec
- run it locally with `npx fireline run`
- package it as a hosted OCI image with `npx fireline build`
- hand that image to target-native tooling with `npx fireline deploy --to <platform>`

Fireline does not introduce its own deployment API. The hosted artifact is an OCI image with the spec embedded, and the deploy step is a thin wrapper over `flyctl`, `wrangler`, `docker compose`, or `kubectl`.

## Current Status On `main`

| Surface | Status | What is honest to claim today |
| --- | --- | --- |
| `npx fireline run agent.ts` | `PASS` | Local run is the cleanest shipped path. It boots Fireline locally, provisions the spec, and prints ACP/state URLs. |
| `npx fireline build agent.ts` | `SHIPPED` | Hosted-image build exists and emits an OCI image plus optional scaffold files. It requires a working Docker daemon. |
| Local Docker hosted-image proof | `PARTIAL PASS` | The embedded-spec OCI image boots locally, exposes ACP, and persists durable-streams data on mounted storage. Full restart-safe `session/load` is still not green after container restart. |
| `npx fireline deploy agent.ts --to fly` | `SHIPPED` | Thin wrapper exists and maps to `flyctl deploy`, but this repo does not yet carry a Fly smoke pass. |
| `npx fireline deploy agent.ts --to cloudflare-containers` | `SHIPPED / NOT DEMO-GREEN` | Wrapper exists and maps to `wrangler deploy`, but the demo lane explicitly deferred Cloudflare Containers pending a better durable-storage story. |
| `npx fireline deploy agent.ts --to k8s` | `SHIPPED` | Thin wrapper exists and maps to `kubectl apply -f`, but this repo does not yet carry a Kubernetes smoke pass. |

If you need the local evidence behind the Docker row, use [docs/reviews/smoke-tier-a-local-docker-2026-04-12.md](../../reviews/smoke-tier-a-local-docker-2026-04-12.md).

## 1. Write One Portable Spec

The CLI path needs a file that default-exports the result of `compose(...)`.

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['claude-acp']),
)
```

Keep this file portable:

- use the default export
- keep deployment choice out of the file
- let `run`, `build`, and `deploy` all consume the same spec

Important current limit:

- examples under `examples/` still mostly call `.start()` imperatively, so they are not direct `fireline run` / `build` / `deploy` inputs yet

## 2. Prove The Spec Locally

```bash
npx fireline run agent.ts
```

Expected output excerpt:

```text
durable-streams ready at http://127.0.0.1:7474/v1/stream

  ✓ fireline ready

    ACP:       ws://127.0.0.1:...
    state:     http://127.0.0.1:7474/v1/stream/fireline-state-runtime-...
```

That is the first portability checkpoint:

- the spec parses
- the control plane provisions it
- the session plane is reachable
- the observation plane is reachable

If this step is not clean, do not move to `build` or `deploy` yet.

## 3. Build The Hosted Image

```bash
npx fireline build agent.ts
```

Expected output excerpt:

```text
fireline: building fireline-agent:latest

  ✓ fireline build complete

    image:     fireline-agent:latest
```

What `build` actually does:

- shells out to `docker build`
- bakes the serialized spec into the hosted Fireline OCI image
- optionally writes one target-specific scaffold file if you pass `--target <platform>`

Two practical notes:

- if Docker is unavailable, `build` stops here
- if the scaffold file already exists (`fly.toml`, `wrangler.toml`, `k8s.yaml`, `docker-compose.yml`, or `Dockerfile`), the CLI refuses to overwrite it

## 4. Pick The Right Target Pair

`build` and `deploy` use different target names on purpose.

| Goal | `fireline build --target` | Scaffold file | `fireline deploy --to` | Native tool |
| --- | --- | --- | --- | --- |
| Fly.io | `fly` | `fly.toml` | `fly` | `flyctl deploy` |
| Cloudflare Containers | `cloudflare` | `wrangler.toml` | `cloudflare-containers` | `wrangler deploy` |
| Kubernetes | `k8s` | `k8s.yaml` | `k8s` | `kubectl apply -f` |
| Docker Compose | `docker-compose` | `docker-compose.yml` | `docker-compose` | `docker compose up -d` |
| Raw Dockerfile scaffold only | `docker` | `Dockerfile` | n/a | run your own Docker workflow |

This is the core product story:

- Fireline builds one hosted image
- each target gets a thin native manifest around that image
- `deploy` is convenience, not a Fireline-specific protocol

## 5. Deploy To Fly

Manual path if you want to inspect the generated manifest:

```bash
npx fireline build agent.ts --target fly
flyctl deploy --config fly.toml --image fireline-agent:latest --remote-only
```

Shortcut if you do not already have `fly.toml` in this directory:

```bash
npx fireline deploy agent.ts --to fly -- --remote-only
```

What the wrapper does:

- rebuilds the hosted image
- writes `fly.toml`
- runs `flyctl deploy --config <path> --image fireline-agent:latest --remote-only`

Expected success excerpt:

```text
✓ fireline deploy complete
  image:     fireline-agent:latest
  target:    fly
```

Status note:

- the wrapper is real on `main`
- this repo does not yet carry a Fly smoke review, so treat this as the shipped CLI path, not a validated durability claim

## 6. Deploy To Cloudflare Containers

Manual path if you want to inspect the generated manifest:

```bash
npx fireline build agent.ts --target cloudflare
wrangler deploy --config wrangler.toml
```

Shortcut if you do not already have `wrangler.toml` in this directory:

```bash
npx fireline deploy agent.ts --to cloudflare-containers
```

What the wrapper does:

- rebuilds the hosted image
- writes `wrangler.toml`
- runs `wrangler deploy --config <path>`

Important honesty note:

- the CLI surface is real
- the demo lane deferred Cloudflare Containers as the proof target pending a better durable-storage story for long-lived Fireline state

So this section is a cookbook for the current wrapper, not a claim that Cloudflare Containers is already demo-green for durable restart behavior.

## 7. Deploy To Kubernetes

Manual path if you want to inspect the generated manifest:

```bash
npx fireline build agent.ts --target k8s
kubectl apply -f k8s.yaml --namespace fireline
```

Shortcut if you do not already have `k8s.yaml` in this directory:

```bash
npx fireline deploy agent.ts --to k8s -- --namespace fireline
```

What the wrapper does:

- rebuilds the hosted image
- writes `k8s.yaml`
- runs `kubectl apply -f <path> --namespace fireline`

The generated manifest is intentionally small:

- one `Deployment`
- one `Service`
- `/healthz` readiness and liveness probes

Treat it as a starting scaffold, not a complete production chart.

## 8. What Has Actually Been Smoke-Tested

The repo's strongest hosted proof today is the local Docker review:

- [docs/reviews/smoke-tier-a-local-docker-2026-04-12.md](../../reviews/smoke-tier-a-local-docker-2026-04-12.md)

The important outcomes from that review:

- the embedded-spec OCI image boots locally
- ACP is reachable on the mapped host port
- a prompt round-trip succeeds
- durable-streams data persists on an attached host volume
- approval state survives a `docker kill` / restart at the stream layer
- full `session/load` after restart still fails, so the complete "unkillable agent in Docker" claim is not ready yet

That is why the honest current demo framing is:

- **GO** for "same hosted image boots locally and preserves durable-streams data"
- **NO-GO** for "restart-safe `session/load` is already green on local Docker"

## 9. Persistent Storage Matters

Hosted Fireline is only as durable as the storage behind its state streams.

On every hosted target, ask the same question:

- where does durable-streams data live between restarts?

Local Docker smoke used:

```bash
-v "$PWD/.tmp/fireline-embedded-spec:/var/lib/fireline"
```

The same idea carries to cloud targets:

- Fly needs attached or equivalent persistent storage
- Kubernetes needs a persistent volume, not only an ephemeral container filesystem
- Cloudflare Containers needs a durable-state story good enough for Fireline's long-lived stream data before it should be presented as a polished target

If you deploy the hosted image onto ephemeral-only storage, you are shipping the container but not the durability story.

## 10. What Could Go Wrong

- `fireline build` requires Docker.
  No daemon means no hosted image.
- `fireline deploy` requires the native platform CLI.
  `flyctl`, `wrangler`, `docker compose`, or `kubectl` must already be installed.
- Existing scaffold files block regeneration.
  The CLI refuses to overwrite `fly.toml`, `wrangler.toml`, `k8s.yaml`, `docker-compose.yml`, or `Dockerfile`.
- Hosted portability is not the same as full restart-proof workflow replay.
  The local Docker smoke review still failed `session/load` after container restart.
- Old portability docs may mention `--provider anthropic --always-on`.
  That is not the current hosted CLI story. The current story is portable OCI image plus target-native deploy.

## Read This Next

- [Quickstart](./quickstart.md)
- [CLI](../cli.md)
- [Providers](../providers.md)
- [docs/reviews/smoke-tier-a-local-docker-2026-04-12.md](../../reviews/smoke-tier-a-local-docker-2026-04-12.md)
- [docs/proposals/hosted-deploy-surface-decision.md](../../proposals/hosted-deploy-surface-decision.md)
- [docs/proposals/hosted-fireline-deployment.md](../../proposals/hosted-fireline-deployment.md)
