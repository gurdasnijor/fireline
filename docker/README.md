# Fireline Host OCI Images

This directory contains the Phase 1 MVP container packaging for the hosted
Fireline control plane from
[`docs/proposals/hosted-fireline-deployment.md`](../docs/proposals/hosted-fireline-deployment.md).

The packaging intentionally ships two variants:

- default: host image plus durable-streams sidecar
- quickstart-only: single image that runs both processes in one container

The sidecar variant is the default recommendation. The single-image variant is
only for quickstarts and small demos.

Tier A boot is image-native: the deployment spec is baked into the host image
at build time and is present in the container at `/etc/fireline/spec.json`.
There is no spec-registration HTTP endpoint in this packaging model.

## Images

| File | Purpose | Exposed ports |
|---|---|---|
| `docker/fireline-host.Dockerfile` | Fireline control-plane host only, with embedded-spec layer | `4440` |
| `docker/fireline-streams.Dockerfile` | Durable-streams sidecar for the host image | `7474` |
| `docker/fireline-host-quickstart.Dockerfile` | Convenience image bundling host + streams, with embedded-spec layer | `4440`, `7474` |

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

`docker compose up --build` intentionally keeps the sidecar topology unchanged.
It will build the host image with the placeholder spec at
`docker/specs/placeholder-spec.json`. For a real deployment, rebuild the host
image with a spec override and then use the same sidecar runtime shape.

## Embedded-spec boot path

Both host images accept a build-time `SPEC` arg. The value must be a path inside
the Docker build context that points to a serialized `compose(...)` Harness
JSON file.

Inside the image, the spec is copied to:

```text
/etc/fireline/spec.json
```

The image also exports:

```text
FIRELINE_EMBEDDED_SPEC_PATH=/etc/fireline/spec.json
```

This is the canonical Tier A OCI-embedded-spec location described by
[`docs/proposals/hosted-deploy-surface-decision.md`](../docs/proposals/hosted-deploy-surface-decision.md)
and
[`docs/proposals/hosted-fireline-deployment.md`](../docs/proposals/hosted-fireline-deployment.md).
The image boot path is "read embedded spec on boot", not "register spec over
HTTP".

### Manual sidecar builds

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -f docker/fireline-host.Dockerfile \
  --build-arg SPEC=./agent.spec.json \
  -t fireline-host-dev:w17 \
  .

docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -f docker/fireline-streams.Dockerfile \
  -t fireline-streams:latest \
  .
```

If you need a local iteration tag outside CI, use a unique tag such as
`fireline-host-dev:w18` to avoid colliding with other work.

If you omit `--build-arg SPEC=...`, the image bakes in
`docker/specs/placeholder-spec.json` so local compose and CI smoke builds keep
working.

## Quickstart: Single Image

The quickstart image runs `fireline` and `fireline-streams` in one container:

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -f docker/fireline-host-quickstart.Dockerfile \
  --build-arg SPEC=./agent.spec.json \
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

Like the sidecar host image, the quickstart image defaults to
`docker/specs/placeholder-spec.json` unless you pass `--build-arg SPEC=...`.

## Runtime configuration

Common environment variables:

| Variable | Default | Meaning |
|---|---|---|
| `FIRELINE_PORT` | `4440` | Fireline host listen port |
| `FIRELINE_HOST` | `0.0.0.0` | Fireline host bind address |
| `FIRELINE_NAME` | `hosted-fireline` | Logical host name |
| `FIRELINE_CONTROL_PLANE_PROVIDER` | `local` | Current control-plane provider mode |
| `FIRELINE_EMBEDDED_SPEC_PATH` | `/etc/fireline/spec.json` | Canonical path to the OCI-embedded deployment spec |
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
