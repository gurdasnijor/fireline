# Fireline Host OCI Images

This directory contains the Phase 1 MVP container packaging for the hosted
Fireline control plane from
[`docs/proposals/hosted-fireline-deployment.md`](../docs/proposals/hosted-fireline-deployment.md).

The packaging intentionally ships two variants:

- default: host image plus durable-streams sidecar
- quickstart-only: single image that runs both processes in one container

The sidecar variant is the default recommendation. The single-image variant is
only for quickstarts and small demos.

## Images

| File | Purpose | Exposed ports |
|---|---|---|
| `docker/fireline-host.Dockerfile` | Fireline control-plane host only | `4440` |
| `docker/fireline-streams.Dockerfile` | Durable-streams sidecar for the host image | `7474` |
| `docker/fireline-host-quickstart.Dockerfile` | Convenience image bundling host + streams | `4440`, `7474` |

All three Dockerfiles are compatible with `linux/amd64` and `linux/arm64`
through `docker buildx`.

## Default: Sidecar Topology

The default deployment shape is one Fireline host container plus one
durable-streams sidecar container with a shared persistent volume:

```bash
docker compose up --build
```

That command uses the repository root [`docker-compose.yml`](../docker-compose.yml)
and starts:

- `fireline-streams` on the internal compose network
- `fireline-host` on `http://127.0.0.1:4440`

The host image defaults `FIRELINE_DURABLE_STREAMS_URL` to
`http://fireline-streams:7474/v1/stream`, which matches the compose service
name. Override it when deploying to another platform.

### Manual sidecar builds

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -f docker/fireline-host.Dockerfile \
  -t fireline-host:latest \
  .

docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -f docker/fireline-streams.Dockerfile \
  -t fireline-streams:latest \
  .
```

If you need a local iteration tag outside CI, use a unique tag such as
`fireline-host-dev:w18` to avoid colliding with other work.

## Quickstart: Single Image

The quickstart image runs `fireline` and `fireline-streams` in one container:

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -f docker/fireline-host-quickstart.Dockerfile \
  -t fireline-host-quickstart:latest \
  .

docker run --rm \
  -p 4440:4440 \
  -p 7474:7474 \
  -v fireline-host-data:/var/lib/fireline \
  fireline-host-quickstart:latest
```

### Durability warning

The quickstart image is not the recommended production topology.

- It couples the Fireline host lifecycle and the durable-streams lifecycle.
- It still requires a persistent volume mounted at `/var/lib/fireline`.
- Running it on ephemeral container storage will lose durable-streams data and
  violate the durability assumptions in the hosted deployment proposal.

If you need a durable hosted control plane, prefer the sidecar topology.

## Runtime configuration

Common environment variables:

| Variable | Default | Meaning |
|---|---|---|
| `FIRELINE_PORT` | `4440` | Fireline host listen port |
| `FIRELINE_HOST` | `0.0.0.0` | Fireline host bind address |
| `FIRELINE_NAME` | `hosted-fireline` | Logical host name |
| `FIRELINE_CONTROL_PLANE_PROVIDER` | `local` | Current control-plane provider mode |
| `FIRELINE_DURABLE_STREAMS_URL` | `http://fireline-streams:7474/v1/stream` | External durable-streams URL for the host-only image |
| `FIRELINE_STREAMS_PORT` | `7474` | Durable-streams sidecar/bundled port |
| `DS_STORAGE__MODE` | `file-durable` | Durable-streams storage mode |
| `DS_STORAGE__DATA_DIR` | `/var/lib/fireline/durable-streams` | Durable-streams storage directory |

The sidecar and quickstart images wrap `fireline-streams` with a tiny TCP proxy
so the container can expose `7474` even though the current Rust helper binds
its HTTP listener on loopback internally.

## CI

`docker/**` changes trigger
[`.github/workflows/managed-agent-suite.yml`](../.github/workflows/managed-agent-suite.yml).
The workflow validates:

- multi-arch `buildx` builds for the host, sidecar, and quickstart images
- compose-based sidecar smoke bring-up
- quickstart image health on both `4440` and `7474`
