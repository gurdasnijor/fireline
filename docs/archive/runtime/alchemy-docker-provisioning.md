# Alchemy Docker Provisioning For Remote Fireline Runtimes

Status: exploration

Related:
- [`provider-lifecycle.md`](./provider-lifecycle.md)
- [`../execution/next-steps-proposal.md`](../execution/next-steps-proposal.md)
- [`../architecture.md`](../architecture.md)
- Alchemy concepts:
  - <https://github.com/georgejeffers/alchemy-skills/blob/main/skills/alchemy/references/alchemy-concepts.md>
  - <https://alchemy.run/providers/docker/>

## Purpose

Describe a concrete first path for remote runtime provisioning where:

- Fireline continues to own the runtime contract
- a control plane continues to own runtime lifecycle decisions
- Alchemy is used strictly as the provisioning layer for the substrate

This doc stays intentionally narrower than a full distributed-runtime design.
It focuses on one provider shape:

- **Docker-backed remote runtimes provisioned through Alchemy**

The goal is to prove a realistic `Remote` provider path without forcing Fireline
to adopt a cloud-specific runtime model too early.

## Why Docker first

Docker is the cleanest first remote provider because it matches Fireline's
current assumptions closely:

- one Fireline runtime is one long-lived process
- that process serves both `/acp` and `/v1/stream/*`
- ACP is reached over a direct WebSocket endpoint
- the embedded durable-streams server can write to a local filesystem path

That makes Docker a better first remote target than a more opinionated hosting
model that adds an extra ingress/runtime layer in front of the process.

## Boundary: what Fireline owns vs what Alchemy owns

This split is the most important design rule.

### Fireline owns

- `RuntimeDescriptor` shape and lifecycle semantics
- `runtimeKey` and `nodeId`
- runtime registration and heartbeat
- ACP and stream auth
- peer discovery and runtime discovery
- mapping infrastructure health into Fireline runtime statuses
- the durable session/state contract

### Alchemy owns

- image build or image selection
- network, volume, and container creation
- secret injection into the container environment
- bootstrapping the Fireline process on a target Docker host
- replacing, restarting, or destroying the container substrate

Alchemy is therefore an implementation detail of the remote provider adapter,
not the source of truth for Fireline runtime identity.

## Mapping Alchemy concepts onto Fireline

This section uses Alchemy's own terminology so the integration matches the
mental model described in their docs.

### App

One Alchemy app should correspond to the Fireline control-plane deployment, not
to an individual runtime.

Example:

- Alchemy app: `fireline-control-plane`

### Stage

An Alchemy stage should correspond to an environment boundary:

- developer stage
- preview / PR stage
- staging
- production

This maps cleanly onto Fireline environments and keeps infra state isolated.

### Scope

Each remote runtime should live in its own nested Alchemy scope keyed by
`runtimeKey`.

That gives every runtime a small, isolated resource graph:

- one network
- one volume
- one image or image reference
- one container

### Resource

The useful abstraction is a custom Alchemy resource such as
`fireline::DockerRuntime`, implemented in terms of Docker provider primitives.

That resource can own:

- Docker `Network`
- Docker `Volume`
- Docker `Image` or `RemoteImage`
- Docker `Container`

### Secret

Alchemy secrets should be used only for provisioning-time and boot-time
materials, for example:

- runtime registration token
- ACP bearer token
- stream bearer token
- control-plane API credentials

They are not a substitute for Fireline's own runtime/session durability.

### State

Alchemy state tracks infrastructure reconciliation.

Fireline's durable stream tracks runtime/session state.

Those are separate systems and should remain separate:

- Alchemy state answers "what infra resources exist?"
- Fireline state answers "what happened inside the runtime?"

## Proposed architecture

```text
client.host.create({ provider: "docker", ... })
  -> Fireline control plane
     -> allocate runtimeKey/nodeId and runtime record
     -> invoke Alchemy deployment for one runtime scope
        -> Docker network
        -> Docker volume
        -> Docker image
        -> Docker container running fireline
     -> wait for Fireline registration + health
     -> publish RuntimeDescriptor
```

At steady state:

```text
Client
  -> ACP / stream URL from RuntimeDescriptor
  -> remote Fireline runtime

Fireline runtime
  -> heartbeats / registration updates
  -> control plane

Alchemy
  -> reconciles container substrate
  -> Docker host
```

## Runtime resource shape

The remote-provider adapter should behave as if there is one logical resource:

- `fireline::DockerRuntime`

Its outputs should be enough to populate a `RuntimeDescriptor`:

- `providerInstanceId`
- `acpUrl`
- `stateStreamUrl`
- optional `helperApiBaseUrl`
- health / readiness metadata

Internally it can be composed from lower-level Docker resources, but callers
should not need to understand that graph.

## Bootstrap contract for the container

To make Docker-backed provisioning work cleanly, Fireline needs a slightly more
explicit bootstrap contract than it has today.

The remote container should receive at least:

- `runtimeKey`
- `nodeId`
- advertised base URL, or separate advertised ACP/stream URLs
- bind host and port
- control-plane registration URL
- registration token
- ACP auth token
- stream auth token
- stream storage mode and data dir
- agent command

This is the main place where the current local-only assumptions need to be
cleaned up:

- advertised URL must be separate from bind address
- `nodeId` must be supplied, not derived from host IP
- local file-backed runtime and peer registries cannot remain the remote source
  of truth

## Create flow

### 1. Control plane allocates runtime identity

The control plane creates a runtime record in `starting` state and assigns:

- `runtimeKey`
- `nodeId`
- provider = `docker`
- desired image / build target
- desired advertised URL

### 2. Control plane generates runtime secrets

At minimum:

- registration token
- ACP bearer token
- stream bearer token

These are injected through Alchemy as secrets.

### 3. Control plane deploys one Alchemy runtime scope

Recommended scope:

- `runtimes/<runtimeKey>`

Within that scope, the Alchemy deployment creates:

- Docker network
- Docker volume for Fireline stream storage
- image reference or image build
- one Docker container running `fireline`

### 4. Container boots Fireline

The container starts `fireline` with explicit bootstrap inputs. For a first
pass, the runtime should bind internally to `0.0.0.0:4437` and use file-backed
stream storage on the mounted volume.

### 5. Fireline self-registers

After boot, the runtime calls a control-plane registration endpoint and
publishes:

- `runtimeKey`
- `runtimeId`
- `nodeId`
- advertised `acpUrl`
- advertised `stateStreamUrl`

The control plane should treat this as the authoritative transition from
provisioned substrate to a ready Fireline runtime.

### 6. Control plane verifies readiness

Readiness should require both:

- successful registration
- a passing health check

Only then should the runtime record move to `ready`.

## Stop, restart, and delete flow

### Stop

Stopping a remote runtime should:

- mark the runtime `stopping` or `stopped`
- stop the container
- keep the runtime record queryable
- preserve the runtime volume by default

Keeping the volume makes restart and debugging much simpler.

### Restart

Restart should reuse the same runtime scope and recreate or start the container
without changing:

- `runtimeKey`
- `nodeId`

The resulting `runtimeId` may change if Fireline treats process incarnation as a
new runtime instance. That is acceptable as long as the runtime record remains
stable.

### Delete

Deleting the runtime should:

- destroy the container
- destroy the network
- remove the runtime record
- optionally destroy or preserve the volume

The volume policy should be explicit, because it is a data-bearing resource even
if the container itself is not.

## Networking model

The simplest first model is:

- one container port exposed for Fireline
- one advertised HTTPS origin per runtime
- both ACP and stream traffic served from the same origin

Examples:

- `https://runtime-123.example.com/acp`
- `https://runtime-123.example.com/v1/stream/fireline-state-...`

This matches Fireline's current single-listener model and avoids introducing a
gateway-specific protocol too early.

## Auth model

The simplest first auth model is bearer tokens.

Recommended split:

- one registration token used only for runtime -> control-plane registration
- one ACP bearer token for `/acp`
- one stream bearer token for `/v1/stream/*`

This keeps the first remote provider operationally simple while leaving room for
mTLS or capability-token refinement later.

## Storage model

For the first Docker-backed remote provider:

- use file-durable stream storage on a mounted Docker volume
- keep that volume outside the container lifecycle

This preserves the current Fireline model where the runtime owns its embedded
durable-streams server while avoiding total state loss on container replacement.

Important boundary:

- Alchemy state is not Fireline runtime state
- Docker volume contents are Fireline's provider-local persistence
- Fireline durable streams remain the consumer-facing durability contract

## Proposed TypeScript surface

This doc is not a committed API, but the remote path likely wants a shape close
to:

```ts
await client.host.create({
  provider: "docker",
  name: "agent-a",
  agentCommand: ["fireline-testy"],
  docker: {
    hostRef: "docker-host-prod-us-west-2a",
    image: "ghcr.io/org/fireline:latest",
    advertisedBaseUrl: "https://agent-a.example.com",
    volumeRetention: "preserve",
  },
})
```

The important point is that the caller requests a Fireline runtime, not a raw
container. Docker-specific fields stay nested under the provider.

## Proposed Alchemy shape

Pseudo-code only:

```ts
import alchemy from "alchemy";
import * as docker from "alchemy/docker";

const app = await alchemy("fireline-control-plane", { stage });

export async function ensureFirelineRuntime(spec: {
  runtimeKey: string;
  nodeId: string;
  image: string;
  advertisedBaseUrl: string;
  controlPlaneUrl: string;
  registrationToken: string;
  acpToken: string;
  streamToken: string;
}) {
  return alchemy.run(`runtimes/${spec.runtimeKey}`, async () => {
    const network = await docker.Network("net", {
      name: `fireline-${spec.runtimeKey}`,
    });

    const volume = await docker.Volume("data", {
      name: `fireline-data-${spec.runtimeKey}`,
    });

    const container = await docker.Container("runtime", {
      image: spec.image,
      name: `fireline-${spec.runtimeKey}`,
      start: true,
      restart: "always",
      ports: [{ internal: 4437, external: 4437 }],
      networks: [{ name: network.name }],
      volumes: [{ hostPath: volume.name, containerPath: "/var/lib/fireline" }],
      environment: {
        FIRELINE_RUNTIME_KEY: spec.runtimeKey,
        FIRELINE_NODE_ID: spec.nodeId,
        FIRELINE_ADVERTISED_BASE_URL: spec.advertisedBaseUrl,
        FIRELINE_CONTROL_PLANE_URL: spec.controlPlaneUrl,
        FIRELINE_REGISTRATION_TOKEN: spec.registrationToken,
        FIRELINE_ACP_TOKEN: spec.acpToken,
        FIRELINE_STREAM_TOKEN: spec.streamToken,
        DS_STORAGE__MODE: "file-durable",
        DS_STORAGE__DATA_DIR: "/var/lib/fireline/streams",
      },
    });

    return {
      providerInstanceId: container.id,
      volumeName: volume.name,
      networkName: network.name,
    };
  });
}

await app.finalize();
```

The exact Docker provider field names may differ; the point is the resource
shape and lifecycle split, not this literal syntax.

## What Fireline would need to add

The minimal Fireline changes implied by this design are:

1. Explicit remote provider kind in the runtime host surface.
2. Explicit `nodeId` input instead of host-derived `nodeId`.
3. Explicit advertised URL config instead of deriving URLs from the bind
   address.
4. Registration and heartbeat path from runtime -> control plane.
5. Remote discovery backend that replaces local file-backed registries.
6. Auth on ACP and stream routes.

These are the same core seams already implied by Direction B; Docker just gives
them a concrete first provider target.

## Why this is a better first slice than Cloudflare-specific hosting

This is not an argument against Cloudflare Containers. It is only a sequencing
argument.

Docker is the more direct first provider because it preserves Fireline's current
process model:

- one runtime process
- one listener
- one embedded stream server
- one mounted local data path

Once those remote-provider seams are clean, a Cloudflare-specific adapter can
sit on top of the same Fireline runtime contract with a different ingress and
lifecycle story.

## Open questions

- Does the first remote provider target a local Docker daemon, a remote Docker
  host, or a provider-managed Docker-compatible service?
- Should runtime stop preserve the data volume by default?
- Should ACP and stream auth use the same bearer token or separate tokens?
- Does the control plane own DNS/TLS, or does the provider adapter own it?
- How does the control plane surface logs and restart reasons for a failed
  container?
- Should `providerInstanceId` be the Docker container id, a stable Alchemy
  resource id, or both?

## Summary

The simplest credible remote-provider path is:

- Fireline defines a stable remote runtime contract
- a control plane allocates runtime identity and desired state
- Alchemy provisions Docker substrate for that runtime
- the Fireline process self-registers and becomes reachable through a normal
  `RuntimeDescriptor`

That keeps the architectural boundary clean:

- Fireline is still the runtime substrate
- Flamecast or another control plane is still the orchestrator
- Alchemy is the provisioning layer underneath the remote provider
