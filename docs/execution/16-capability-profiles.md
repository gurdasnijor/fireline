# 16: Capability Profiles

Status: planned
Type: execution slice

Related:

- [`../product/capability-profiles.md`](../product/capability-profiles.md)
- [`../product/object-model.md`](../product/object-model.md)
- [`../product/product-api-surfaces.md`](../product/product-api-surfaces.md)
- [`../product/priorities.md`](../product/priorities.md)
- [`../product/roadmap-alignment.md`](../product/roadmap-alignment.md)
- [`./14-runs-and-sessions-api.md`](./14-runs-and-sessions-api.md)
- [`./15-workspace-object.md`](./15-workspace-object.md)
- [`./13-distributed-runtime-fabric/13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md`](./13-distributed-runtime-fabric/13a-control-plane-runtime-api-and-external-durable-stream-bootstrap.md)
- [agent.pw](https://github.com/smithery-ai/agent.pw)

## Objective

Prove the first-cut `CapabilityProfile` product object so Fireline has one
portable answer to:

- which MCPs and tools a run should carry
- where credentials come from
- which reusable instructions apply
- which policy defaults apply

This slice should establish the profile as the reusable environment preset that
sits above runtime placement and above workspace identity.

The scope should stay intentionally narrow:

- first-cut schema
- credentials as references only
- compile profile defaults into existing substrate surfaces

## Product Pillar

Portable capability profiles.

## User Workflow Unlocked

Users and host products can:

- reuse the same MCP/tool environment across many runs
- reference credentials indirectly instead of injecting raw secrets
- apply instruction and policy defaults consistently
- start equivalent runs across local and later remote runtimes without
  rebuilding the agent environment every time

## Why This Slice Exists

Without a profile model, Fireline will keep scattering agent-environment
concerns across:

- runtime bootstrap config
- local config files
- topology snippets
- UI-only remembered choices
- provider-specific environment injection

That makes reuse, portability, and safe credential handling much harder than
they need to be.

## Scope

### 1. CapabilityProfile object and schema

Define a first-cut `CapabilityProfile` product object with stable identity and
portable semantics.

Required first-cut fields:

- `profileId`
- `name`
- `description?`
- `mcpServers`
- `credentialRefs`
- `instructionLayers`
- `policies`
- `defaults?`
- `createdAtMs`
- `updatedAtMs`

Required first-cut defaults:

- `agentId?`
- `topology?`
- `placementMode?`

### 2. Product API surface

Add first-cut product-layer profile APIs:

```ts
client.profiles.list()
client.profiles.get(profileId)
client.profiles.create(spec)
client.profiles.update(profileId, patch)
client.profiles.clone(profileId, overrides?)
```

The product surface should treat profiles as reusable environment presets, not
as provider-specific manifests.

### 3. Credentials as references, never payloads

This slice must make one important boundary explicit:

- profiles store credential references
- profiles do not store raw tokens or secrets

Allowed first-cut examples:

- secret ids
- vault paths
- `agent.pw` connection paths
- credential scope descriptors

Disallowed:

- raw OAuth access tokens
- provider-specific secret env payloads as the profile source of truth

### 4. Compilation into existing substrate surfaces

This slice should define how a profile compiles into the already-existing
systems layer.

At minimum:

- MCP bindings compile into run/bootstrap config
- instruction layers compile into context/topology defaults
- policy defaults compile into topology and gate defaults
- preferred agent defaults compile into catalog/run-start defaults

The profile layer should sit above the substrate, not replace it.

### 5. Run integration

Runs should be able to reference `profileId` explicitly.

This slice should make explicit:

- how a run starts from `workspace + profile + agent + placement`
- which defaults a run inherits from the profile
- which fields a run may still override directly

### 6. First-cut provider neutrality

The first version of the profile must remain portable across placement modes.

That means the schema should avoid embedding:

- local-only filesystem assumptions
- container-only lifecycle settings
- provider-specific resource manifests

## Explicit Non-Goals

This slice does **not** require:

- a secret-storage system inside Fireline
- replacing `agent.pw`
- per-provider CPU/memory sizing policy
- full inheritance trees or organization-wide profile distribution
- a rich dynamic policy language
- putting all run configuration into profiles

## Acceptance Criteria

- `CapabilityProfile` exists as an explicit product object with a first-cut
  reusable schema
- `client.profiles.list/get/create/update/clone` exist
- credential handling is reference-based only; raw secret payloads are not part
  of the profile contract
- a run can reference `profileId` and inherit profile defaults
- profile concerns compile into honest existing substrate surfaces rather than
  inventing a second hidden execution model
- one product consumer can treat a profile as "the environment this run should
  have" rather than reconstructing MCPs, credentials, and policy ad hoc

## Validation

- `cargo test -q`
- `pnpm --filter @fireline/client test`
- one TypeScript integration test that:
  - creates a first-cut profile
  - starts multiple runs that reference that profile
  - verifies inherited MCP or topology defaults are applied
  - verifies credentials remain references, not inline secret payloads
- one consumer-oriented integration test that:
  - inspects a run
  - shows which profile it used
  - shows which defaults were inherited from that profile

## Handoff Note

Keep this slice narrow and portable.

Do not:

- make the profile a provider manifest
- make Fireline a secret vault
- inline credentials into runtime bootstrap as the profile source of truth
- solve every future policy problem here

The key proof is:

- a profile is the reusable capability/policy preset for a run
- credentials travel by reference
- the profile compiles down into existing Fireline substrate surfaces cleanly

