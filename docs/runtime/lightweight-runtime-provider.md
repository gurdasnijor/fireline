# Lightweight Runtime Provider

Status: exploration

Related:
- [`provider-lifecycle.md`](./provider-lifecycle.md)
- [`alchemy-docker-provisioning.md`](./alchemy-docker-provisioning.md)
- [`agent-catalog-and-launch.md`](./agent-catalog-and-launch.md)
- [`../research/agent-os.md`](../research/agent-os.md)
- [`../research/agent-os-overview.md`](../research/agent-os-overview.md)

## Purpose

Describe a Fireline runtime-provider model that borrows the useful orchestration
patterns from agentOS without copying the in-process kernel or guest-OS design.

The target outcome is:

- lightweight runtimes that feel like a real agent execution environment
- imperative Rust-side lifecycle control
- provider adapters that can target Docker first, with room for Kubernetes,
  SSH-managed hosts, or heavier sandboxes later

## Core idea

Fireline should treat remote runtime hosting as a runtime-orchestration problem,
not primarily as an infrastructure-reconciliation problem.

That means:

- a control plane asks Fireline to create a runtime
- a Fireline host-side runtime manager creates and manages that runtime
- provider adapters translate that request into container/process operations
- the resulting runtime registers and becomes reachable through a stable
  `RuntimeDescriptor`

The provider layer should feel more like a lightweight runtime orchestrator than
an IaC engine.

## What To Borrow From agentOS

agentOS is useful prior art here, but mainly for system boundaries and runtime
shape rather than for its kernel model.

### 1. Bootstrap and attach are separate

The strongest pattern to borrow is:

- runtime/bootstrap owns creation
- ACP attaches to an already-owned runtime

That maps directly onto Fireline:

- `client.host` owns create/get/list/stop/delete
- `client.acp` connects to a provided endpoint or transport

### 2. Trait-first provider boundary

agentOS's bridge contract is transport-agnostic and I/O-free. Fireline should
copy that pattern for runtime providers.

The important design move is:

- define the provider contract in Rust
- keep Docker/Kubernetes/SSH specifics behind that trait
- keep control-plane transport and client APIs outside the provider logic

### 3. Base environment plus writable overlay plus mounts

This is the most transferable runtime-environment pattern.

agentOS models:

- a base filesystem snapshot
- a per-runtime writable overlay
- explicit mounted filesystems

For Fireline, the container/process equivalent is:

- a prepared base image or runtime root
- a per-runtime writable workspace layer or volume
- explicit mounts for workspace, caches, secrets, and optional extensions

This is enough to give the agent a "proper runtime environment" without
building a whole kernel.

### 4. Ownership-scoped cleanup

agentOS uses ownership scopes and cleanup cascades. Fireline should adopt the
same lifecycle discipline:

- host or node owns runtimes
- runtime owns sessions
- session owns attachments or helper resources

When a parent is disposed, children are cleaned up deterministically.

### 5. Sequenced events and resumable observation

Runtime lifecycle should be observable through a sequenced event stream:

- created
- booting
- registered
- ready
- busy
- unhealthy
- stopped
- deleted

This helps both operators and TS consumers resume observation without replay
ambiguity.

### 6. Escalation to a heavier environment on demand

agentOS's "sandbox extension" idea is worth copying conceptually.

Fireline should support:

- a lightweight default runtime path
- an optional escalation path when the workload needs a fuller environment

Examples:

- browser automation
- native daemons
- nested containers
- long-lived dev servers

The default provider should stay cheap; heavier environments should be opt-in.

## What Not To Borrow

### 1. Not the in-process kernel

Fireline should not try to virtualize:

- VFS
- process tables
- sockets
- PTYs
- Node builtins

Containers or real host processes already provide a sufficiently "proper"
execution environment for Fireline's purpose.

### 2. Not the vertically integrated product shape

agentOS is much more vertically integrated. Fireline should keep the split
clear:

- Fireline is the runtime substrate
- Flamecast or another control plane is the orchestrator above it

### 3. Not the guest polyfill strategy

agentOS has to intercept and emulate large parts of the guest runtime API. That
complexity is unnecessary if Fireline chooses real containers or real host
processes as the runtime substrate.

## Proposed Fireline Shape

The proposed architecture is:

```text
Control plane
  -> runtime manager on a host/node
     -> RuntimeProvider trait
        -> DockerProvider
        -> KubernetesProvider
        -> SshProvider
        -> SandboxProvider

Runtime provider
  -> prepares environment
  -> starts fireline runtime
  -> waits for registration
  -> returns provider instance metadata
```

The key shift is that Fireline gains a host-side runtime manager process or
service that owns runtime lifecycle imperatively.

## Runtime Manager

Each host or node should run one Fireline runtime manager responsible for:

- creating runtimes
- starting and stopping runtimes
- collecting logs
- reporting health
- cleaning up leaked runtimes
- managing prepared images, caches, and warm pools

This is the closest Fireline analogue to the orchestration role agentOS gives to
its sidecar.

## Runtime Environment Model

The runtime environment should be an explicit first-class concept.

### Base runtime

Each provider should start from a prepared base environment that already
contains:

- the `fireline` binary
- a compatible terminal agent or agent launcher path
- standard shell utilities
- expected directories
- CA certificates and basic networking tools

For containers this is a base image. For process-based providers this may be a
prepared host directory or packaged runtime bundle.

### Writable layer

Each runtime needs a writable layer for:

- durable-streams local storage
- runtime-local caches
- temporary files
- optional workspace data

This should be separate from the base environment so the runtime can be started
quickly and disposed cleanly.

### Mounts

Provider adapters should expose explicit mount kinds rather than open-ended host
access.

Useful first mounts:

- workspace mount
- cache mount
- secret projection
- tool cache mount
- artifact/output mount

Future mounts:

- remote object-store mount
- sandbox mount
- browser profile mount

### Capability profile

Every runtime should declare a capability profile, for example:

- `minimal`
- `coding`
- `browser`
- `sandbox`

This influences:

- network access
- filesystem mounts
- resource limits
- tool availability
- provider selection

### Limits

The runtime manager should always set explicit bounds:

- CPU and memory
- max disk usage for writable storage
- runtime idle timeout
- max concurrent attachments
- max logs retained locally

## Suggested Provider Trait

The provider seam should be imperative and Rust-native.

Pseudo-shape:

```rust
trait RuntimeProvider {
    type InstanceId;
    type Error;

    async fn create(&self, spec: RuntimeSpec) -> Result<ProvisionedRuntime<Self::InstanceId>, Self::Error>;
    async fn inspect(&self, instance: &Self::InstanceId) -> Result<RuntimeHealth, Self::Error>;
    async fn logs(&self, instance: &Self::InstanceId) -> Result<RuntimeLogStream, Self::Error>;
    async fn stop(&self, instance: &Self::InstanceId) -> Result<(), Self::Error>;
    async fn start(&self, instance: &Self::InstanceId) -> Result<(), Self::Error>;
    async fn delete(&self, instance: &Self::InstanceId) -> Result<(), Self::Error>;
}
```

`RuntimeHost` remains the stable Fireline-facing lifecycle surface. Providers are
internal implementations behind it.

## Suggested RuntimeSpec

The spec should describe a Fireline runtime environment, not raw container
mechanics.

Pseudo-shape:

```rust
struct RuntimeSpec {
    runtime_key: String,
    node_id: String,
    provider_hint: RuntimeProviderRequest,
    agent_launch: AgentLaunchSpec,
    environment: RuntimeEnvironmentSpec,
    registration: RegistrationSpec,
    auth: RuntimeAuthSpec,
}

struct RuntimeEnvironmentSpec {
    profile: RuntimeProfile,
    base_image: Option<String>,
    mounts: Vec<RuntimeMountSpec>,
    caches: Vec<RuntimeCacheSpec>,
    limits: RuntimeLimits,
    idle_timeout_ms: Option<u64>,
}
```

The provider then maps that Fireline-native shape into container, pod, or host
process configuration.

## Docker First

The first provider should target Docker or Podman because it already matches the
current Fireline runtime model well:

- one runtime is one long-lived process
- one process serves `/acp` and `/v1/stream/*`
- one writable local data path is enough for embedded stream storage
- real shells and subprocesses already work naturally inside the container

This is likely the lowest-complexity path to a "proper enough" runtime
environment.

A Rust Docker API client is a better fit here than an IaC layer because the
operations are imperative:

- start this runtime now
- wait for registration
- tail logs
- stop and delete it

## Warm Pools And Prepared Runtimes

One pattern worth borrowing strongly from agentOS is minimizing cold-start work
by separating base preparation from per-runtime creation.

For Fireline, that means:

- pre-pull or pre-build base images
- pre-create reusable caches
- optionally keep a small warm pool of stopped or idle prepared runtimes
- create new runtimes by attaching a fresh writable layer rather than building
  from scratch

This gives most of the latency win without adopting agentOS's in-process kernel.

## Sandboxes As An Extension Provider

A heavier sandbox should be treated as an extension or alternate provider, not
the default.

That means the lightweight provider path stays simple, while workloads that need
more can escalate to:

- E2B
- Daytona
- browser-focused sandboxes
- Kubernetes pods with broader privileges

This mirrors the agentOS idea that lightweight execution and heavyweight
sandboxes can coexist rather than compete.

## Observability

The runtime manager should expose host-level observation that complements
Fireline's durable state stream.

Useful runtime-manager events:

- runtime requested
- provider create started
- provider create finished
- runtime registered
- health degraded
- restart attempted
- stop requested
- runtime deleted

This is operational metadata, not user/session durability data.

## Security Posture

The lightweight provider should default to explicit, narrow runtime inputs:

- explicit mounts, not whole-host access
- explicit env vars and secret projections
- explicit network mode
- explicit resource limits

For Docker-first providers this likely means:

- curated base image
- bind mounts only where necessary
- bearer-token auth on ACP and stream surfaces
- non-root runtime user where possible

## Implementation Sequence

### 1. Clean up Fireline bootstrap assumptions

Before provider expansion, Fireline needs:

- explicit advertised URLs
- explicit `nodeId`
- route auth
- registration/heartbeat flow

### 2. Add provider trait and host runtime manager

Move provider lifecycle behind a trait and stop assuming local process spawn is
the only runtime shape.

### 3. Implement DockerProvider

The first provider should support:

- image selection
- writable volume
- workspace mount
- log collection
- health polling
- start/stop/delete

### 4. Add runtime environment profiles

Introduce provider-neutral environment descriptions such as `minimal`,
`coding`, and `browser`.

### 5. Add heavier providers later

Once the runtime contract is stable:

- KubernetesProvider
- SshProvider
- SandboxProvider

## Why This Is Better Than Leading With IaC

IaC is still useful for:

- creating base hosts
- networking
- DNS
- TLS
- cluster setup

But it is not the best primary abstraction for Fireline runtime lifecycle.

Runtime lifecycle needs:

- immediate create/start/stop/delete
- health polling
- log streaming
- cleanup of leaked instances
- close coupling to Fireline registration state

That is a much better match for a Rust runtime-manager + provider-trait design.

## Open Questions

- Is the runtime manager an embedded mode of `fireline` or a separate host-side
  daemon?
- Do we want a warm pool in the first Docker provider or only after the core
  flow works?
- What is the smallest useful base image that still feels like a proper coding
  environment?
- Should workspace mounts be required, optional, or profile-dependent?
- Does a runtime always map to one agent process, or can a provider host
  multiple runtimes in one shared container?
- How much host-level observation should be surfaced through `RuntimeDescriptor`
  versus a separate host-ops API?

## Summary

The agentOS patterns worth carrying forward are:

- bootstrap/attach separation
- trait-first orchestration boundaries
- prepared base environments plus writable overlays and mounts
- ownership-scoped cleanup
- sequenced lifecycle events
- optional escalation to heavier sandboxes

The key thing not to copy is the kernel.

For Fireline, a lightweight runtime provider should be:

- Rust-native
- imperative
- Docker-first
- capable of presenting a real runtime environment
- able to escalate to heavier providers only when needed
