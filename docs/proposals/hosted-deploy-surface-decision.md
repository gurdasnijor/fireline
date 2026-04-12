# Hosted Deploy Surface — Decision

> Status: architectural decision
> Date: 2026-04-12
> Author: Architect (Opus 3)
> Blocks: [hosted-fireline-deployment.md](./hosted-fireline-deployment.md) Phase 1, [fireline-cli-execution.md](./fireline-cli-execution.md) Phase 1

## TL;DR

**No new HTTP control-plane endpoint is introduced.** Spec registration against a hosted Fireline uses two existing substrates, tiered by scale:

- **Tier A (primary, MVP): OCI-embedded spec.** `fireline build` emits an image with the spec baked in. Deploy is target-native (`wrangler deploy`, `fly deploy`, `kubectl apply`, `docker run`). The host boots, reads the embedded spec, starts serving. Zero new surface.
- **Tier C (secondary, multi-spec): durable-streams resource.** Specs live on a namespaced resource stream (`specs:tenant-{id}`). The host runs a `DeploymentSpecSubscriber` (a DurableSubscriber profile) that observes the stream and materializes deployments. `fireline push <spec> --to <stream-url>` is a thin durable-streams `append`; no new protocol.

Both tiers preserve the user's conceptual split:

1. **Providers** = runtime substrate (docker, local, Anthropic managed, microsandbox) — unchanged.
2. **Sandboxes** = per-session execution containers — unchanged.
3. **Deploy target** = OCI packaging + target-native tooling — NOT a Fireline-specific protocol.

## Topology

```text
┌────────────────────────────────────────────────────────────────┐
│ TIER A: OCI-embedded spec (MVP / single-spec deployments)     │
└────────────────────────────────────────────────────────────────┘

  developer laptop                 cloud target (any)
  ─────────────────                ─────────────────────
  agent.ts                         ┌─────────────────┐
       │                           │ container       │
       │ fireline build            │  ├── fireline   │
       ▼                           │  │   host       │
  OCI image                        │  ├── embedded   │
   (spec embedded)                 │  │   spec       │
       │                           │  └── durable-   │
       │ wrangler / fly /          │      streams    │
       │ kubectl / docker          │      (sidecar)  │
       ▼                           └─────────────────┘
  target cloud ─────► host boots, reads embedded spec, serves

┌────────────────────────────────────────────────────────────────┐
│ TIER C: Durable-streams spec resource (multi-spec / live)     │
└────────────────────────────────────────────────────────────────┘

  developer laptop                 cloud target (any)
  ─────────────────                ─────────────────────
  agent.ts                         ┌─────────────────┐
       │                           │ fireline host   │
       │ fireline push             │                 │
       │  --to <stream-url>        │  DeploymentSpec │
       ▼                           │  Subscriber     │◄──┐
  durable-streams ─────────────────│  (DurableSub-   │   │ observes
   append to                       │   scriber      )│   │ specs:tenant
   specs:tenant-{id}               │                 │   │ stream
                                   │  materializes   │   │
                                   │  sandboxes per  │   │
                                   │  active spec    │   │
                                   └─────────────────┘   │
                                          │              │
                                          └──────────────┘
```

Both tiers: durable-streams state plane is the truth for session/permission/chunk state (unchanged from canonical-identifiers proposal). Providers and sandboxes sit under the host. No new HTTP surface in either tier.

## Evaluation Against Candidates

### (a) OCI-embedded spec — **ACCEPTED as Tier A primary**

- Zero new API surface.
- Matches user's OCI-first framing exactly: "deploying is just `wrangler deploy` / `fly deploy` / `kubectl apply` — target-native tooling."
- `alwaysOn` lives IN the embedded spec; target-platform's own keepalive / replica / machine-class policy satisfies it.
- Limitation: one spec per image; changing spec = rebuild + redeploy.
- Tradeoff acceptable: single-spec is the dominant MVP case and matches the `docs/demos/pi-acp-to-openclaw.md` narrative ("same 15-line file moves from local to cloud").

### (b) Spec as an ACP session — **REJECTED**

ACP is a runtime protocol (session lifetime = conversation lifetime), not a deployment protocol. "Always-on deployment" and "ACP session" have distinct lifetimes; collapsing them requires either extending ACP or abusing `_meta`. Either would muddy the managed-agent boundary, which is precisely what the user is asking us to protect.

### (c) Spec as a durable-streams resource — **ACCEPTED as Tier C secondary**

- No new HTTP surface. Specs go on `specs:tenant-{id}`, consumed by a `DeploymentSpecSubscriber`.
- `DeploymentSpecSubscriber` is a new DurableSubscriber profile — but that's a profile on an existing primitive, not a new primitive. It composes with the already-specified `AlwaysOnDeploymentSubscriber` (deployment_wake_requested → sandbox_provisioned).
- Handles multi-tenant, multi-spec, and live updates naturally.
- `fireline push <spec>` is a thin durable-streams `append` wrapper. The verb is honest: it's an append, not a deploy.
- Auth model: same as existing durable-streams auth (per-tenant write credentials). No new auth concept.

### (d) Spec via agent catalog — **REJECTED**

`agent_catalog` names AGENT identities (`pi-acp`, `claude-sonnet-4-6`) for reuse across deployments. Conflating "the agent's identity" with "a running deployment instance" muddies the catalog. Also the catalog is conceptually pull/read-mostly; making it end-user-writable shifts its semantics more than (c) does.

### (e) New HTTP endpoint — **REJECTED**

Neither (a) nor (c) requires one. Introducing `PUT /v1/deployments/{name}` or `PUT /v1/specs/{name}` would create a third control plane alongside ACP and durable-streams, for no behavioral capability the other two don't already provide.

## `npx fireline deploy` — Concrete Verbs

Rescoped to match the tiered model. No CLI verb implies a new HTTP protocol.

- **`fireline build <spec> [--target <platform>]`**
  Emits an OCI image with the spec embedded. If `--target` is provided, optionally scaffolds target-specific config files (`wrangler.toml`, `fly.toml`, `Dockerfile`, `k8s.yaml`). Pure codegen; no network calls.

- **`fireline deploy <spec> --to <platform>`** (thin target-adapter)
  Convenience wrapper: `fireline build` + `wrangler deploy` (or `fly deploy`, `kubectl apply`, `docker run`). Delegates to target-native tooling. Not a Fireline protocol; a UX shortcut for the target's own deploy command. Can be dropped if it creates more confusion than it removes.

- **`fireline push <spec> --to <stream-url>`** (Tier C only)
  Appends the spec to the given durable-streams resource. Host's `DeploymentSpecSubscriber` observes and materializes. Equivalent to `curl -X POST` against durable-streams append — just typed.

**Deleted from the CLI surface:**

- `fireline deploy agent.ts --target production` where `production` implies a Fireline-owned control plane — gone. The `--target` in `fireline deploy` now means a platform name (`cloudflare`, `fly`, `kubernetes`), not a Fireline deployment slot.
- Any CLI verb that `PUT`s a spec at a Fireline HTTP endpoint — never lands.

## `lifecycle.alwaysOn` Placement

Answer: **spec metadata, consumed by whichever subscriber applies.**

- Tier A: field on the spec embedded in the image. Host reads on boot. Target platform's keepalive / replica configuration is the enforcement substrate (`fly.toml [services.http_checks]`, CF Containers keepalive, K8s `replicas: 1`).
- Tier C: field on the spec resource envelope. `AlwaysOnDeploymentSubscriber` (already specified) observes it, materializes and keeps sandbox alive per the wake invariants.

Not in ACP `_meta` (ACP is runtime, not deployment). Not a separate field. Spec-level metadata, read by the appropriate subscriber.

## Impact on Existing Proposals

- **`hosted-fireline-deployment.md`** Phase 1 must reframe as: "host boots, reads embedded spec or subscribes to spec stream — no HTTP control plane added." The phased plan can still enumerate target platforms; the deploy MECHANISM changes from "PUT spec to host" to "target-native OCI deploy."
- **`fireline-cli-execution.md`** Phase 1 must reframe `fireline deploy` as either (a) build+scaffold or (b) push-to-stream. Drop any implied HTTP PUT surface.
- **`durable-subscriber.md` / `durable-subscriber-execution.md`** gets one new profile: `DeploymentSpecSubscriber` (alongside `AlwaysOnDeploymentSubscriber`). Both are profiles on the existing primitive; no new primitive.
- **`durable-subscriber-verification.md`** should cover `DeploymentSpecSubscriber` replay (spec stream idempotent under replay, no duplicate materialization) under the existing `DSV-01` + `DSV-02` invariants. No new invariant IDs needed.

## Rationale

The user's architectural instinct is right: every new HTTP surface is debt. The managed-agent / ACP / durable-streams trio already gives Fireline a control plane; adding a deployment API would create a parallel seam that has to be justified every time it's touched, and tends to grow features that should have been modeled as agent-plane or stream-plane behavior.

OCI-embedded spec is the smallest possible deploy story and maps 1:1 to how real developers think about container deployment. Durable-streams spec resource is the cleanest extension when one-image-per-spec is not enough, and it composes with the DurableSubscriber primitive the codebase is already building. Both tiers preserve plane separation: specs are infrastructure-plane inputs; running agents are agent-plane.

## Acceptance

- [ ] Architect signs off on the tiered model.
- [ ] PM marks `hosted-fireline-deployment.md` Phase 1 as gated on this document landing.
- [ ] PM marks `fireline-cli-execution.md` Phase 1 as gated on this document landing.
- [ ] Subscriber catalog gains `DeploymentSpecSubscriber` profile entry (Tier C only; not required for Tier A MVP).

## References

- [hosted-fireline-deployment.md](./hosted-fireline-deployment.md)
- [fireline-cli-execution.md](./fireline-cli-execution.md)
- [deployment-and-remote-handoff.md](./deployment-and-remote-handoff.md)
- [durable-subscriber.md](./durable-subscriber.md)
- [durable-subscriber-execution.md](./durable-subscriber-execution.md)
- [acp-canonical-identifiers.md](./acp-canonical-identifiers.md)
- [docs/demos/pi-acp-to-openclaw.md](../demos/pi-acp-to-openclaw.md)
