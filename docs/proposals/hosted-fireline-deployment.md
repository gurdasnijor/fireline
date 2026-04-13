# Hosted Fireline Deployment

Status: proposal
Date: 2026-04-12
Scope: infrastructure plan for the hosted Fireline instance that boots from an embedded spec (Tier A) or, later, a spec stream subscription (Tier C) and runs long-lived deployed agents against a co-located durable-streams service.

## 1. Architectural decisions

This proposal does not reopen the decisions already recorded in [`docs/status/orchestration-status.md`](../status/orchestration-status.md).

Phase 1 of this document is gated on the tiered deploy model decision in [`hosted-deploy-surface-decision.md`](./hosted-deploy-surface-decision.md) (`77e007d`). Tier A is the MVP boot path: the deployment spec is baked into the OCI image at build time and read on host boot. Tier C is deferred: specs arrive via a durable-streams resource and are materialized by `DeploymentSpecSubscriber`. No Fireline-owned deployment HTTP API is introduced in either tier.

1. Fireline Host ships as a portable OCI image. The deployment target is any cloud container platform that can run a long-lived HTTP/SSE service and attach or colocate durable storage for durable-streams.
2. Durable-streams is co-located with the hosted Fireline control plane. It is not embedded in sandboxes and it must outlive sandbox crashes, host restarts, and provider churn.
3. Anthropic managed agents remain the primary cloud sandbox provider for the demo and first hosted path. `microsandbox`, Docker, and local subprocess remain secondary providers behind the existing provider abstraction.

Always-on deployment behavior is not a separate primitive. It is spec metadata consumed by the DurableSubscriber substrate, specifically the `AlwaysOnDeploymentSubscriber` profile described by [`durable-subscriber.md`](./durable-subscriber.md), to turn `deployment_wake_requested` into `sandbox_provisioned`.

### 1.1 Host runtime packaging decision

| Choice | Why it wins | Why the alternatives lost for this proposal |
|---|---|---|
| Portable OCI image | One artifact across hosted targets, clean `npx fireline deploy` story, consistent local/CI/cloud packaging | A Fly-specific or provider-specific artifact would couple product design to one target and fight the new cloud-agnostic direction |
| Target-specific adapters | Lets deploy stay neutral while each platform handles ingress, secrets, and storage its own way | Baking target behavior into the runtime itself would fragment the host model |

### 1.2 Durable-streams placement decision

| Choice | Why it wins | Why the alternatives lost for this proposal |
|---|---|---|
| Co-located durable-streams | Preserves low-latency stream access, clean failure-domain reasoning, and durable state outside every sandbox | Embedding durable-streams in each sandbox violates the durability and plane-separation requirements |
| Sidecar as default; same-image as quickstart-only | Keeps durable-streams adjacent to the host while making the safer production topology the default | Treating same-image and sidecar as co-equal recommendations would hide the durability and upgrade risks of the bundled variant |

### 1.3 Default provider decision

| Choice | Why it wins | Why the alternatives lost for this proposal |
|---|---|---|
| Anthropic managed agents as default | Strongest hosted demo path, least Fireline-owned sandbox lifecycle for the first cloud story, already implemented behind the provider abstraction | Leading with Docker or microsandbox makes the hosted narrative depend on Fireline owning code-exec infrastructure on day one |
| Secondary providers remain available | Preserves the multi-provider model and keeps full code-exec available when needed | Making Anthropic exclusive would undercut the provider abstraction and the self-hosted story |

## 2. Portable OCI Image + Target Matrix

The product artifact is one OCI image:

- `ghcr.io/fireline/hosted-fireline:<tag>`

That image contains:

- the Fireline host process
- the provider dispatcher
- the Anthropic provider client
- the microsandbox-based runtime image base and entrypoint wiring
- optional embedded durable-streams binary for same-image mode
- readiness, health, and migration entrypoints

`npx fireline deploy` should never build a platform-specific runtime. It should build or reference the same OCI image and publish platform-specific deployment metadata around it.

### 2.1 Durable-streams co-location options

| Option | Shape | Advantages | Tradeoffs | Best fit |
|---|---|---|---|---|
| Sidecar | Fireline host container + durable-streams sidecar in one service/task/pod | Better fault isolation, cleaner metrics/logs split, easier future separation | Requires platform support for multi-container services or pod/task equivalents | Default topology for hosted Fireline |
| Same image | Fireline host and durable-streams run in one container image | Simplest artifact story, easiest local parity, one deploy unit | Harder lifecycle isolation, upgrades couple host and stream server, weaker observability split | Quickstart convenience only for demos, simple VMs, and small self-hosted installs |

Default recommendation: sidecar. The bundled same-image variant is a quickstart convenience, not the recommended production topology.

> Warning
>
> The bundled same-image variant requires attached persistent storage at the platform level. Ephemeral container storage is not acceptable. If the image is recycled without a preserved disk or volume, durable-streams state is lost and `SessionDurableAcrossRuntimeDeath` is violated.

### 2.2 Target matrix

| Target | Fit for hosted Fireline | Persistent storage story | SSE smoke-test status | Operational notes |
|---|---|---|---|---|
| Cloudflare Containers | First-class target for portable OCI distribution and global ingress | Today container disk is ephemeral, so durable-streams cannot rely on local disk there; use external co-located durable-streams or revisit when persistent disk exists | Pending | Excellent routing and image portability, but not the easiest first bootstrap for durable-streams-backed hosted Fireline today |
| Fly.io | First-class target and strongest bootstrap candidate | Local persistent volumes are available per machine and fit durable-streams well | Pending | Good match for long-lived SSE services and multi-region placement; volume-backed instances trade off easy zero-downtime swaps |
| Railway | First-class target | Volumes exist, but replicas cannot share them and deploys with volumes incur small downtime | Pending | Strong DX, simpler than Kubernetes, acceptable for a single-region MVP |
| Render | First-class target | Persistent disks exist, but only for a single instance and they disable zero-downtime deploys | Pending | Good managed-service ergonomics; similar single-instance constraints as Railway for the durable-streams node |
| Self-hosted Docker Compose | First-class target | Named volume or host bind mount | Pending | Good for small hosted installs, demos, and operator-managed single-node deployments |
| Kubernetes | First-class target | PVC/StatefulSet for durable-streams, Deployment/StatefulSet for host | Pending | Most flexible and most work; not the MVP bootstrap target |
| Bare VM with Docker | First-class target | Host filesystem or attached block volume mounted into the container | Pending | Strong escape hatch for operators that want no platform abstraction |

### 2.3 Validation contract

No target enters the supported list until it passes the same ACP SSE smoke test:

- start hosted Fireline on that target
- provision one Anthropic-backed deployment
- hold an ACP SSE session open long enough to cross normal proxy and platform idle windows
- verify prompt, approval, and reconnect behavior over the same hosted instance
- restart or recycle the host image and verify durable-stream resumption semantics still hold

Initial state for every target in this proposal: `Pending`.

### 2.4 Bootstrap target

Phase 1 should bootstrap on Fly.io, not because Fireline is Fly-specific, but because Fly is the cleanest currently available combination of:

- portable OCI deployment
- long-lived HTTP/SSE service support
- persistent local volume for durable-streams
- multi-region path for the later peer-fleet phases

Cloudflare Containers stays first-class in the design, but not the Phase 1 bootstrap target because durable-streams currently needs persistent storage that Cloudflare Containers does not yet provide on local container disk.

## 3. Service Topology

```mermaid
flowchart LR
    SPEC[agent.ts / compose spec] --> BUILD[fireline build]
    BUILD --> REG[OCI registry]
    REG --> HOSTA[Hosted Fireline host image<br/>region A]
    REG --> HOSTB[Hosted Fireline host image<br/>region B]

    subgraph REGIONA[Region A]
      HOSTA --> EMBED_A[embedded spec<br/>Tier A]
      HOSTA --> DS_A[durable-streams<br/>sidecar default]
      HOSTA --> ANTH_A[Anthropic provider]
      HOSTA --> SEC_A[tenant secrets / env bridge]
      SPEC_A[specs:tenant-{id}<br/>Tier C] --> SUB_A[DeploymentSpecSubscriber]
      SUB_A --> HOSTA
    end

    subgraph REGIONB[Region B]
      HOSTB --> EMBED_B[embedded spec<br/>Tier A]
      HOSTB --> DS_B[durable-streams<br/>sidecar default]
      HOSTB --> ANTH_B[Anthropic provider]
      HOSTB --> SEC_B[tenant secrets / env bridge]
      SPEC_B[specs:tenant-{id}<br/>Tier C] --> SUB_B[DeploymentSpecSubscriber]
      SUB_B --> HOSTB
    end

    DS_A <-- hosts:tenant-{id} --> DS_B
    DS_A <-- state/session/{session_id} --> HOSTA
    DS_B <-- state/session/{session_id} --> HOSTB
    HOSTA <-- peer discovery --> HOSTB
    OP[operator UI / admin API] --> HOSTA
```

Key boundaries:

- Agent-plane state lives in `state/session/{session_id}` streams.
- Infrastructure-plane state lives in tenant-scoped discovery and registry streams such as `hosts:tenant-{id}` and sandbox inventory streams.
- Provider backends do not own durable truth. They consume provisioning instructions and emit observable events back into Fireline-managed state.
- Tier A boot reads the spec embedded in the OCI image. Tier C boot reads a spec resource via `DeploymentSpecSubscriber`. Both tiers use the same durable-streams-backed state plane once the host is up.
- Default production topology is host container plus durable-streams sidecar. Bundled single-image mode is a quickstart variant only.

## 4. Boot Paths and Deployment Pipeline

Hosted Fireline keeps deploy transport out of its runtime contract. Tier A boots from an embedded spec inside the OCI image. Tier C boots from a spec stream watched by `DeploymentSpecSubscriber`. Neither tier introduces a Fireline-owned deploy HTTP control plane.

### 4.1 Tier A boot path: OCI image + embedded spec

1. Load `agent.ts` and `fireline.config.ts`.
2. Resolve the target environment and provider override.
3. Compile the declarative agent spec to the hosted Fireline deployment manifest:
   - agent topology
   - middleware chain
   - sandbox provider default
   - secrets references
   - peer declarations
   - tenant/namespace metadata
4. Bake that manifest into the OCI image layer at build time.
5. Push the image to a registry reachable by the target platform.
6. Use target-native tooling to run the image:
   - `fly deploy`
   - `docker run`
   - `kubectl apply`
   - equivalent platform-native deploy commands
7. The target platform pulls the image and starts the hosted Fireline service.
8. On boot, the host reads the embedded spec, initializes or attaches durable-streams, and registers itself in infra-plane discovery state.
9. Hosted deploys stay warm by default via the `AlwaysOnDeploymentSubscriber` substrate. Cold-start opt-out is not in scope for the initial ship.

### 4.2 Tier C boot path: spec stream subscription (deferred)

1. The hosted image boots with `DeploymentSpecSubscriber` enabled.
2. A spec resource is appended to a tenant-scoped durable stream such as `specs:tenant-{id}`.
3. `DeploymentSpecSubscriber` materializes or updates the deployment intent from that stream.
4. The host reconciles that intent against provider state using the same sandbox inventory and discovery streams as Tier A.
5. Hosted deploys stay warm by default via the `AlwaysOnDeploymentSubscriber` substrate. Cold-start opt-out is not in scope for the initial ship.

Tier C is intentionally deferred until the durable-subscriber profile is landed and replay-safe. Phase 1 does not depend on it.

### 4.3 Artifacts

| Artifact | Produced by | Consumed by |
|---|---|---|
| Embedded deployment spec | `compose()` / CLI build step | Hosted Fireline boot path inside the OCI image |
| OCI image | CLI build or CI pipeline | Registry + target platform |
| Platform deploy descriptor | CLI target adapter | Fly/Railway/Render/Cloudflare/K8s/Docker |
| Tenant secret bindings | target adapter + secret manager | Hosted Fireline runtime |
| Durable-streams config | target adapter | sidecar or embedded stream process |
| Tier C spec resource | `fireline push` or direct durable-streams append | `DeploymentSpecSubscriber` |

### 4.4 Platform-neutral rule

The build/deploy flow publishes the same hosted image everywhere. Platform adapters change only:

- how the image is launched
- how persistent storage is attached
- how secrets are injected
- how ingress/TLS is terminated

They do not change the Fireline runtime model.

## 5. Operational Concerns

### 5.1 Cold start

- Anthropic is the default cloud provider because it minimizes cold-start work inside Fireline itself.
- Hosted Fireline still has two cold paths: host process boot and sandbox wake.
- Always-on deployments use the DurableSubscriber wake path to pre-provision sandboxes before user traffic hits them.

### 5.2 Failover

- Host restarts must not lose agent-plane state because durable-streams is external to every sandbox and persistent across host crashes.
- If durable-streams is sidecar-backed on local disk, failover is regional and explicit.
- If the platform cannot guarantee attached persistent local storage, durable-streams must run as a separate co-located service rather than in the same container.
- The bundled same-image variant is only valid when the platform attaches a persistent disk or volume to that container. Without that attachment, image recycle destroys durable state and the hosted deployment is invalid.

### 5.3 Multi-region

- Region-local hosts announce themselves via infra-plane discovery streams.
- Cross-region peer routing uses the discovery proposal already defined in [`cross-host-discovery.md`](./cross-host-discovery.md).
- Session ownership remains flat and durable. A region change is an infrastructure event, not an agent-identity change.

### 5.4 Logs and metrics

- Host logs, provider logs, and durable-streams logs should remain separable even in same-image mode.
- OTel spans and ACP `_meta.traceparent` propagation remain the cross-service trace spine.
- Audit retention should preserve deployment actions, approval resolutions, and admin operations separately from agent transcript data.

### 5.5 Upgrades

- Phase 1 may accept stop-and-resume maintenance windows for the hosted instance.
- Managed upgrades in later phases should rely on the DurableSubscriber wake invariants already modeled for session durability and single-winner wake semantics.
- Blue-green is preferred when the target platform supports separate volume handoff cleanly; otherwise halt/resume with durable wake is acceptable.

## 6. Security

- Inbound TLS terminates at the target platform edge or load balancer.
- Credentials enter the hosted runtime through target-managed secret injection and are consumed by `secretsProxy`, not raw application env passthrough.
- Tenant isolation applies at three layers:
  - API auth and deployment ownership
  - durable-streams namespace separation
  - provider credential and secret scoping
- Audit retention must cover:
  - deploy / destroy / scale actions
  - provider provisioning attempts
  - approval resolution actions
  - secret binding changes

## 7. Phased Rollout

### Phase 1 — OCI image + embedded-spec boot path

**Scope**

- Gate note: depends on [`hosted-deploy-surface-decision.md`](./hosted-deploy-surface-decision.md) (`77e007d`)
- Bootstrap on Fly.io
- Portable OCI image produced and deployed with the spec embedded at build time
- Durable-streams co-located on persistent volume
- Anthropic provider only
- No deploy HTTP endpoint; host boots by reading the embedded spec

**Gate**

- CI builds the hosted image
- Smoke test: deploy one agent, receive SSE traffic, restart host, session stream survives

**Risks**

- Volume-coupled deploy downtime
- Initial secret-binding ergonomics
- Hosted auth surface still maturing

**Done-when**

- A single tenant can deploy an Anthropic-backed agent to a hosted Fireline instance via target-native OCI deploy, and reconnect after host restart without transcript loss

### Phase 2 — Tier C spec-stream deployments

**Scope**

- Land `DeploymentSpecSubscriber` as a DurableSubscriber profile
- Accept deployment specs from tenant-scoped durable streams such as `specs:tenant-{id}`
- Materialize the same hosted runtime model from stream-backed specs instead of embedded specs
- Reuse the same sidecar-default durable-streams topology and Anthropic-primary provider story

**Gate**

- Replay-safe subscriber smoke test proves a streamed spec materializes exactly once under replay and host restart

**Risks**

- Duplicate materialization under subscriber replay
- Operator confusion between embedded-spec and stream-backed deploy sources

**Done-when**

- Tier C can stand up the same deployment model as Tier A by replaying a spec stream into `DeploymentSpecSubscriber`

### Phase 3 — Multi-region host fleet

**Scope**

- Add peer discovery across regions
- Split ingress from region-local host registration
- Validate remote handoff and peer routing under region loss
- If Tier C is enabled, assign `DeploymentSpecSubscriber` ownership per region without duplicate winners

**Gate**

- Two-region smoke test with cross-host peer discovery and session continuity; Tier C, if enabled, must keep subscriber replay and ownership deterministic

**Risks**

- Misrouting during partial region outage
- Discovery freshness and staleness windows

**Done-when**

- Hosted Fireline can run a region pair with discovery-backed peer routing and explicit failover procedures

### Phase 4 — Secondary sandbox providers

**Scope**

- Enable microsandbox, Docker, and local-subprocess backends as supported hosted options
- Keep Anthropic as the default hosted provider

**Gate**

- Provider conformance smoke suite passes on at least Anthropic + one code-exec provider

**Risks**

- Provider capability skew
- Filesystem and networking assumptions diverge by provider

**Done-when**

- Hosted Fireline can provision the same declarative deployment against Anthropic or one secondary provider without API shape changes

### Phase 5 — Managed upgrades

**Scope**

- Introduce rolling or blue-green upgrades where platform support allows it
- Otherwise formalize halt/resume maintenance using DurableSubscriber wake semantics
- For Tier C, keep `DeploymentSpecSubscriber` replay-safe across host replacement and restart

**Gate**

- Upgrade smoke test proves sessions survive host replacement and always-on deployments rehydrate without duplicate winners, whether the source of truth is an embedded spec or a replayed spec stream

**Risks**

- Split-brain wake attempts
- Volume handoff complexity on platforms with single-instance disks

**Done-when**

- Hosted upgrades are an operational routine, not a manual one-off playbook

### Phase 6 — Observability, billing, fleet ops

**Scope**

- Tenant usage metering
- Fleet inventory
- deployment history
- hooks for the future fleet UI
- Record whether a deployment was booted from an embedded spec or a Tier C spec stream

**Gate**

- Admin API exposes enough host/sandbox/deployment state for operator dashboards without reading agent-plane streams directly, including Tier A vs Tier C provenance

**Risks**

- Cost attribution drift across providers
- Infra-plane schema churn

**Done-when**

- Hosted Fireline has a credible fleet-operations surface for operators and billing systems

## 8. Validation Checklist

- [ ] Hosted Fireline is described as a portable OCI image, not a provider-specific binary
- [ ] Phase 1 explicitly depends on the tiered deploy surface decision in [`hosted-deploy-surface-decision.md`](./hosted-deploy-surface-decision.md) (`77e007d`)
- [ ] Durable-streams is explicitly outside every sandbox and survives sandbox/host crashes
- [ ] Sidecar is the default durable-streams topology and same-image mode is clearly marked quickstart-only
- [ ] Same-image and sidecar durable-streams modes are both documented with tradeoffs
- [ ] The same-image warning explicitly calls out the persistent-disk requirement and the `SessionDurableAcrossRuntimeDeath` failure mode
- [ ] Every listed target carries an ACP SSE smoke-test status and the validation contract says `Pending` is not support
- [ ] Anthropic is the primary hosted sandbox provider, with secondary providers preserved
- [ ] The document explicitly states Tier A boots from an embedded OCI spec and Tier C boots from `DeploymentSpecSubscriber`
- [ ] HTTP deploy endpoints are retired from this proposal
- [ ] Always-on behavior is spec metadata delegated to DurableSubscriber rather than a new primitive
- [ ] The rollout phases have gates, risks, and done-when criteria
- [ ] The document remains cloud-provider-agnostic even though Phase 1 picks one bootstrap target

## 9. Architect Review Checklist

- [ ] Does the OCI-first and embedded-spec framing preserve the intended product story for `fireline build` plus target-native deploy?
- [ ] Are the sidecar and same-image durable-streams options separated clearly enough?
- [ ] Is the bootstrap target choice pragmatic without turning the design into a single-platform plan?
- [ ] Does the hosted topology preserve plane separation between agent state and infrastructure state?
- [ ] Is Anthropic-primary plus multi-provider-secondary the right default narrative for the hosted demo?
- [ ] Is the Tier C `DeploymentSpecSubscriber` story deferred cleanly enough that Phase 1 stays minimal?
- [ ] Are the phase gates crisp enough to dispatch as follow-on execution work?

## Notes

- Cloudflare Containers docs currently describe container disk as ephemeral and persistent disk as future work, which is why Cloudflare is first-class in the matrix but not the bootstrap target for durable-streams-backed hosting today.
- Fly, Railway, and Render all document local persistent storage options, but Railway and Render explicitly constrain replica/zero-downtime behavior when a volume is attached. That makes them viable targets, but with stricter upgrade tradeoffs than a stateless host tier.
