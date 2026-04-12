# Cross-Host Discovery Over Durable Streams

> **Status:** authoritative proposal
> **Type:** design doc
> **Audience:** Host, Tools, browser-harness, and verification workstreams
> **Related:**
> - [`../investigations/stream-backed-peer-discovery.md`](../investigations/stream-backed-peer-discovery.md)
> - [`./deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md)
> - [`./client-primitives.md`](./client-primitives.md)
> - [`./runtime-host-split.md`](./runtime-host-split.md)
> - `verification/spec/managed_agents.tla`

This proposal makes one structural claim:

**Fireline already ships its discovery plane.** The durable-streams service
that every Host already needs for session durability is also the right place to
publish Host presence and runtime presence. Cross-Host discovery is therefore
not a new service, not a new primitive, and not a control-plane feature. It is
one more stream-projected view over infrastructure that already exists.

This doc supersedes the transitional conclusion in
[`../investigations/stream-backed-peer-discovery.md`](../investigations/stream-backed-peer-discovery.md)
that kept `LocalPeerDirectory` as a temporary direct-host fallback. The user
explicitly rejected that fallback. The replacement is stream-backed everywhere.

## 1. TL;DR

- Durable Streams is already Fireline's shared durability plane. This proposal
  makes it the discovery plane too by introducing one per-tenant stream:
  `hosts:tenant-<tenant_id>`. `[User confirmed]`
- Hosts self-publish both Host presence and runtime presence onto that stream.
  Readers project the stream into an in-memory `DeploymentIndex`; no central
  aggregator exists.
- `PeerRegistry` stays the consumer-facing Tools abstraction, but both current
  implementations disappear: `LocalPeerDirectory` is deleted as an architecture
  violation, and `ControlPlanePeerRegistry` is deleted as redundant. `[User confirmed]`
- `GET /v1/runtimes` stops being the discovery surface. The durable stream is
  the discovery surface. Callers that still need runtime status by key can keep
  a separate status path, but discovery itself no longer goes through HTTP.
- A companion TLA spec,
  `verification/spec/deployment_discovery.tla`, should model the discovery
  stream exactly the same way `managed_agents.tla` models wake/session
  semantics: append-only log, replayed projection, explicit invariants.

## 2. The insight

This was easy to miss because Fireline's durable-streams work started as
"session durability" and the peer-discovery story started as a local bootstrap
convenience. Those looked like separate concerns, so discovery leaked into a
local file (`LocalPeerDirectory`) and then into a Host HTTP adapter
(`ControlPlanePeerRegistry`). Structurally that was wrong. Discovery is not a
new primitive in the taxonomy from [`./client-primitives.md`](./client-primitives.md);
it is the concrete implementation of the existing Host + Tools seam from
[`./runtime-host-split.md`](./runtime-host-split.md). Once every Host is
already connected to one durable-streams service, the simplest correct design
is to make the stream the sole authority for state, discovery, and eventually
ACL-enforced reachability. One failure mode, one scaling axis, one health
check, one place to replay from.

## 3. The stream shape

### 3.1 Scope and naming

Discovery is tenant-scoped, never global.

- Stream name: `hosts:tenant-<tenant_id>` `[User confirmed]`
- Cross-tenant discovery: unsupported `[User confirmed]`
- Event split: **Option A, one stream for everything**. Host presence and
  runtime presence live on the same stream unless future volume proves that a
  companion `runtimes:tenant-<id>` stream is necessary. `[User confirmed]`

The stream is append-only. Multiple Hosts append to it concurrently. Readers
replay from offset 0 and then stay live.

### 3.2 Identity rules

- `host_id` is a fresh stable identifier for one Host process lifetime. It
  must not be a machine id or node id. A restarted Host gets a new `host_id`.
- `runtime_key` is the logical runtime identity already used elsewhere in
  Fireline. A runtime can move across Hosts over time; the latest
  `runtime_provisioned` event wins.
- `tenant_id` is carried by stream selection, not duplicated in every event.
- A Host must emit `host_registered` before it emits any `runtime_provisioned`
  events for that `host_id`.

### 3.3 Event schemas

Every event on `hosts:tenant-<tenant_id>` is a single JSON object with a
`kind` discriminator and one of the following bodies.

```rust
pub enum DeploymentDiscoveryEvent {
    HostRegistered {
        host_id: String,
        acp_url: String,
        state_stream_url: String,
        capabilities: serde_json::Map<String, serde_json::Value>,
        registered_at_ms: i64,
        node_info: serde_json::Map<String, serde_json::Value>,
    },
    HostHeartbeat {
        host_id: String,
        seen_at_ms: i64,
        load_metrics: serde_json::Map<String, serde_json::Value>,
        runtime_count: i64,
    },
    HostDeregistered {
        host_id: String,
        reason: String,
        deregistered_at_ms: i64,
    },
    RuntimeProvisioned {
        host_id: String,
        runtime_key: String,
        acp_url: String,
        agent_name: String,
        provisioned_at_ms: i64,
    },
    RuntimeStopped {
        host_id: String,
        runtime_key: String,
        stopped_at_ms: i64,
    },
}
```

#### `host_registered`

```json
{
  "kind": "host_registered",
  "host_id": "host:6d5627d2-8b57-42a6-80c1-6f86f9b2b05e",
  "acp_url": "ws://host-a.example.internal/acp",
  "state_stream_url": "https://streams.example.internal/state/tenant-demo",
  "capabilities": {
    "peerCalls": true,
    "sandboxProvider": "microsandbox",
    "sharedState": true
  },
  "registered_at_ms": 1775904000000,
  "node_info": {
    "region": "us-west-2",
    "version": "fireline/0.1.0"
  }
}
```

- Emitted once per Host boot, after local bootstrap is complete and the Host is
  actually reachable.
- `state_stream_url` is the canonical durable-streams URL another Host should
  use when it needs the advertised Host's shared state surface. Consumers that
  only need ACP peering can ignore it.

#### `host_heartbeat`

```json
{
  "kind": "host_heartbeat",
  "host_id": "host:6d5627d2-8b57-42a6-80c1-6f86f9b2b05e",
  "seen_at_ms": 1775904005000,
  "load_metrics": {
    "cpuPct": 0.34,
    "rssMb": 412
  },
  "runtime_count": 3
}
```

- Best-effort liveness hint, not source of truth.
- Emitted on a fixed cadence by the Host that owns `host_id`.
- Missing heartbeats do not mutate the stream; they only affect the reader's
  freshness calculation.

#### `host_deregistered`

```json
{
  "kind": "host_deregistered",
  "host_id": "host:6d5627d2-8b57-42a6-80c1-6f86f9b2b05e",
  "reason": "graceful_shutdown",
  "deregistered_at_ms": 1775904060000
}
```

- Emitted on graceful shutdown.
- Crash-only failure does not emit this event; stale-heartbeat collapse covers
  that case.

#### `runtime_provisioned`

```json
{
  "kind": "runtime_provisioned",
  "host_id": "host:6d5627d2-8b57-42a6-80c1-6f86f9b2b05e",
  "runtime_key": "runtime:specialist-agent",
  "acp_url": "ws://host-a.example.internal/runtimes/runtime:specialist-agent/acp",
  "agent_name": "specialist-agent",
  "provisioned_at_ms": 1775904010000
}
```

- Emitted only after the runtime is actually ready to accept ACP traffic.
- `acp_url` may equal the Host ACP URL or may be runtime-specific.
- Because the key is `runtime_key`, this event also models runtime migration:
  a later `runtime_provisioned` can move the same logical runtime onto a
  different `host_id`.

#### `runtime_stopped`

```json
{
  "kind": "runtime_stopped",
  "host_id": "host:6d5627d2-8b57-42a6-80c1-6f86f9b2b05e",
  "runtime_key": "runtime:specialist-agent",
  "stopped_at_ms": 1775904055000
}
```

- Emitted when a runtime stops cleanly.
- Readers only remove the runtime if the event's `host_id` matches the
  runtime's current projected owner. That prevents an old stop event from
  deleting a runtime that has already been reprovisioned elsewhere.

### 3.4 Ordering contract

Within one Host, the intended write order is:

1. `host_registered`
2. zero or more `host_heartbeat`
3. zero or more `runtime_provisioned`
4. zero or more `runtime_stopped`
5. `host_deregistered`

The important property is not "every graceful shutdown emits all five kinds."
The important property is that **all durable knowledge is representable as a
stream replay**. Crash paths are therefore represented by omission plus stale
collapse, not by synthetic compensating writes.

## 4. The projection: `DeploymentIndex`

This proposal mirrors the projection pattern already used by
`SessionIndex` and `RuntimeIndex` in `fireline-session`, but it applies it to a
tenant-scoped discovery stream rather than a runtime state stream.

```rust
pub struct DeploymentIndex {
    hosts: HashMap<HostId, HostEntry>,
    runtimes: HashMap<RuntimeKey, RuntimeEntry>,
    stale_threshold_ms: u64,
}

pub struct HostEntry {
    host_id: HostId,
    acp_url: String,
    state_stream_url: String,
    capabilities: Capabilities,
    registered_at_ms: i64,
    last_seen_ms: i64,
    last_heartbeat_metrics: serde_json::Map<String, serde_json::Value>,
    runtime_count: usize,
    node_info: serde_json::Map<String, serde_json::Value>,
    deregistered_at_ms: Option<i64>,
}

pub struct RuntimeEntry {
    runtime_key: RuntimeKey,
    host_id: HostId,
    acp_url: String,
    agent_name: String,
    provisioned_at_ms: i64,
}
```

Freshness is derived, not stored durably:

```rust
impl DeploymentIndex {
    pub fn host_is_fresh(&self, host_id: &HostId, now_ms: i64) -> bool {
        let Some(host) = self.hosts.get(host_id) else { return false };
        host.deregistered_at_ms.is_none()
            && now_ms - host.last_seen_ms < self.stale_threshold_ms as i64
    }

    pub fn list_fresh_runtime_peers(&self, now_ms: i64) -> Vec<Peer> {
        self.runtimes
            .values()
            .filter(|runtime| self.host_is_fresh(&runtime.host_id, now_ms))
            .filter_map(|runtime| self.peer_for_runtime(runtime))
            .collect()
    }
}
```

### 4.1 Apply rules

Readers subscribe live to `hosts:tenant-<tenant_id>`, replay from the
beginning, and apply each event incrementally.

- `host_registered`
  - upsert `HostEntry`
  - set `registered_at_ms = last_seen_ms = registered_at_ms`
  - clear `deregistered_at_ms`
- `host_heartbeat`
  - if `host_id` exists, update `last_seen_ms`, `load_metrics`, and
    `runtime_count`
  - if `host_id` does not exist, ignore the heartbeat
- `host_deregistered`
  - mark the Host absent
  - remove any `RuntimeEntry` whose current `host_id` matches the event
- `runtime_provisioned`
  - if the parent `host_id` is not present, ignore the event
  - otherwise upsert `RuntimeEntry` by `runtime_key`
  - later events replace earlier ones, which is what makes runtime migration a
    normal upsert rather than a special case
- `runtime_stopped`
  - remove the runtime only if `runtime_key` exists and its current `host_id`
    matches the event's `host_id`

### 4.2 What `list()` means

`PeerRegistry`'s consumer-facing contract remains runtime-centric:
`list_peers()` returns runtimes, not Hosts. Host entries exist in the projection
because runtime discovery is Host-dependent and because future Host-level
handoffs need them, but the current MCP surface still wants:

```rust
pub struct Peer {
    pub runtime_id: String,
    pub agent_name: String,
    pub acp_url: String,
    pub state_stream_url: Option<String>,
    pub registered_at_ms: i64,
}
```

The projection maps a `RuntimeEntry` into a `Peer` like this:

- `runtime_id`: the stable exposed identifier becomes `runtime_key`
  until or unless the Tools surface is renamed; no local file lookup remains
- `agent_name`: from `runtime_provisioned.agent_name`
- `acp_url`: from `runtime_provisioned.acp_url`
- `state_stream_url`: from the parent `HostEntry.state_stream_url`
- `registered_at_ms`: from `runtime_provisioned.provisioned_at_ms`

That is a deliberate semantic cleanup. The durable discovery contract is about
reachable logical runtimes, not about local process-instance ids.

### 4.3 Derived truth, never side state

Readers never write to the discovery stream. Writers never mutate readers.
There is no side cache that can disagree with the stream. If the in-memory
projection cannot be rebuilt by replaying from offset 0, the design is wrong.

## 5. Who writes what

### 5.1 Writers

- **Hosts self-publish their own existence.** A Host emits
  `host_registered` on boot, `host_heartbeat` every `N` seconds, and
  `host_deregistered` on graceful shutdown. `[User confirmed]`
- **Hosts self-publish their runtimes.** When a Host provisions a runtime and
  that runtime is actually ready, it emits `runtime_provisioned`. When the
  runtime stops, it emits `runtime_stopped`. `[User confirmed]`
- **There is no central writer.** The stream is the aggregator. `[User confirmed]`

### 5.2 Concrete writer responsibilities

| Moment | Writer | Event |
|---|---|---|
| Host boot completes, ACP is reachable | Host bootstrap path | `host_registered` |
| Heartbeat interval fires | Host bootstrap path | `host_heartbeat` |
| Direct-host runtime becomes ready | direct-host bootstrap path | `runtime_provisioned` |
| Managed runtime create returns ready | managed Host create/provision path | `runtime_provisioned` |
| Runtime stops cleanly | owning Host | `runtime_stopped` |
| Graceful Host shutdown | Host bootstrap path | `host_deregistered` |

Implementation-wise this means the writer logic belongs in the two places that
already own lifecycle:

- `crates/fireline-host/src/bootstrap.rs` for direct-host boot and shutdown
- the managed Host lifecycle path (`crates/fireline-host/src/router.rs` and/or
  the deeper runtime-launch layer it delegates to) for control-plane-managed
  create/stop

### 5.3 Bootstrapping the stream

Every Host should ensure `hosts:tenant-<tenant_id>` exists before its first
append. This is the same shape as existing named-stream bootstrapping:
construct the stream URL, ensure it exists, then append the registration event.

### 5.4 Failure semantics

- Graceful shutdown: `host_deregistered` and `runtime_stopped` make the Host
  disappear immediately.
- Crash / partition: no explicit deregistration happens; the Host naturally
  falls out of discovery once `now_ms - last_seen_ms >= stale_threshold_ms`.
- Runtime migration: a later `runtime_provisioned` for the same `runtime_key`
  simply changes ownership in the projection.

## 6. The `PeerRegistry` collapse

The consumer-facing abstraction stays:

- keep the existing `PeerRegistry` trait as the Tools-level seam
- swap both concrete implementations for one stream-backed satisfier:
  `StreamDeploymentPeerRegistry`

Proposed surface:

```rust
pub struct StreamDeploymentPeerRegistry {
    tenant_id: String,
    stream_url: String,
    stale_threshold_ms: u64,
    poll_interval_ms: u64,
    index: DeploymentIndex,
}
```

Behavior:

- on startup, subscribe live to `hosts:tenant-<tenant_id>`
- replay from offset 0 into `DeploymentIndex`
- on `list_peers()`, return only runtimes whose parent Host is still fresh
- on `lookup_peer(agent_name)`, search only the fresh projected set

This fixes the earlier cross-primitive leakage:

- Tools no longer read local TOML as if it were deployment truth
- Tools no longer round-trip through Host HTTP just to rediscover a stream-
  derivable runtime list

### 6.1 File move / delete table

| Current | Action |
|---|---|
| `crates/fireline-tools/src/peer/directory.rs::LocalPeerDirectory` | delete |
| `crates/fireline-host/src/control_plane_peer_registry.rs::ControlPlanePeerRegistry` | delete |
| `crates/fireline-tools/src/peer/stream.rs::StreamDeploymentPeerRegistry` | NEW |
| `crates/fireline-tools/src/peer/mod.rs` | add `pub mod stream;`, remove `pub mod directory;` |

Additional note: because `directory.rs` currently also owns `Peer` and
`PeerRegistry`, those type definitions should move into `peer/mod.rs` and be
re-exported from `crates/fireline-tools/src/lib.rs`. The file table above is
the authoritative implementation cut; the public path cleanup follows from it.

## 7. The `/v1/runtimes` endpoint collapse

The old HTTP runtime-list surface should stop being discovery.

### 7.1 Recommendation

Choose **option (b)** from the dispatch:

- delete the `GET /v1/runtimes` discovery read path
- make discovery consumers read `hosts:tenant-<tenant_id>` directly, either
  through `StreamDeploymentPeerRegistry` or through a direct stream reader
- do **not** keep a compatibility shim backed by the old file-backed
  `RuntimeRegistry` `[User confirmed]`

This proposal is only about discovery, not about create/stop. `POST /v1/runtimes`
can remain the provision verb. The important delete is the read path that
pretends Host-local runtime state is the right source for fleet discovery.

### 7.2 Callers found

Current `GET /v1/runtimes` list callers in the repo:

| Caller | Why it exists today | Migration |
|---|---|---|
| `crates/fireline-host/src/control_plane_peer_registry.rs` | HTTP-backed peer discovery | delete outright |
| `packages/client/src/host.ts::list()` | legacy control-plane client surface | migrate to stream read or delete if unused |
| `tests/control_plane_docker.rs` | exercises list endpoint | rewrite around stream discovery or remove |
| `tests/support/managed_agent_suite.rs::list_runtimes()` | test helper around control-plane list | rewrite around stream discovery or remove |

Important non-callers:

- `packages/browser-harness` does **not** depend on `GET /v1/runtimes` list.
  Its smoke path uses `POST /cp/v1/runtimes`, `GET /cp/v1/runtimes/{key}`, and
  `POST /cp/v1/runtimes/{key}/stop`.
- `packages/client/src/host-fireline/client.ts` and
  `crates/fireline-orchestration/src/lib.rs` poll `GET /v1/runtimes/{key}` for
  status, not the list endpoint.

That distinction matters. This proposal authoritatively deletes the **list as
discovery surface**. Any later decision to replace `GET /v1/runtimes/{key}`
status polling with a pure stream-backed handle status can happen on its own
lane.

## 8. TLA invariants for `deployment_discovery.tla`

This doc does not write TLA. It specifies what the companion model should
check, using the same naming style as `verification/spec/managed_agents.tla`.

- **HostRegisteredIsEventuallyDiscoverable** — if Host A emits
  `host_registered` at offset `N`, any reader that has replayed past `N`
  observes Host A in its `DeploymentIndex`.
- **HostDeregisteredIsEventuallyInvisible** — if Host A emits
  `host_deregistered` at offset `M`, any reader that has replayed past `M`
  observes Host A as absent.
- **StaleHeartbeatCollapsesToInvisible** — if `now_ms - last_seen_ms`
  exceeds `stale_threshold_ms`, the Host is not fresh and is filtered out of
  `list()`.
- **NoSplitBrainWithinReader** — one reader never observes a single `host_id`
  as both present and absent at the same logical instant; stream order gives a
  total causal sequence.
- **RuntimeDependentOnHost** — `runtime_provisioned` only counts if its
  `host_id` is present in the projected Host map.
- **StreamIsSoleSourceOfTruth** — no `DeploymentIndex` state exists that is
  not reconstructable by replaying the stream from offset 0.

Two additional invariants are worth modeling because they are where migration
and failure semantics get subtle:

- **RuntimeStopOnlyAffectsCurrentOwner** — a `runtime_stopped` event only
  removes a runtime if its `host_id` matches the runtime's current projected
  owner.
- **HostRemovalRemovesHostedRuntimes** — once a Host is deregistered, all
  runtimes currently projected onto that Host are absent from the reader's
  visible peer set.

## 9. Comparison to traditional service discovery

| System | What it gives you | What Fireline gets here instead |
|---|---|---|
| Consul | dedicated service-discovery cluster, KV, health checks | no extra cluster; discovery shares the same durable log already required for session state |
| etcd | consensus-backed registry and watch surface | append-only stream replay is enough because Fireline only needs durable ordered publication + projection |
| DNS SRV | static/distributed name resolution | inadequate for per-runtime lifecycle and no replayable history |
| Kubernetes Services | cluster-local discovery abstraction | useful inside k8s, but Fireline runs across laptop + cloud + non-k8s nodes too |
| Service mesh | auth, policy, routing, observability | far broader than the problem; discovery here is a stream read, not a sidecar platform |

The punchline is not "durable streams is a better Consul." The punchline is
"Fireline already depends on one shared append-only service, so using that same
service for discovery is cheaper and more coherent than adding another control
plane."

## 10. The "holy cow" demo beat

The keynote moment is no longer just "a session moved." It is "a laptop-hosted
agent discovered a cloud-hosted agent it had never seen before, using nothing
but one stream read." Agent A is running on my laptop Host. Mid-conversation it
realizes a cloud specialist is better suited for the next subtask. Agent A
reads `hosts:tenant-demo`, sees a fresh `runtime_provisioned` row for Agent B,
and opens ACP directly to Agent B's `acp_url`. No operator configuration
changed. No peer-directory file was pre-shared. No control-plane registry was
queried.

The reveal is that the same infrastructure already carrying session state is
also carrying deployment truth. Agent B can answer because its Host is
discoverable through the same durable-streams service, and the handoff is
legible because both Hosts are reading and writing durable state to the same
tenant-scoped substrate. The user sees one conversation continue across a
network boundary, but architecturally the deeper message is stronger: **the
Hosts are stateless, the stream is the truth, and discovery is just projection.**

This should replace §6's current demo narrative in
[`./deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md) with
a broader A2A migration story. That parallel doc update is a separate lane; the
authoritative narrative shape lives here.

## 11. What this does not solve

**Authentication.** Out of scope here. Discovery composes with auth rather than
solving it: durable-streams ACLs gate who can read/write
`hosts:tenant-<tenant_id>`, and the planned `SecretsInjectionComponent` handles
credential injection above that. `[User confirmed]`

**NAT traversal.** If Host A advertises a private `acp_url` that Host B cannot
reach, discovery still "works" but connectivity fails. That is a deployment
problem, not a discovery-model problem.

**Cross-tenant federation.** Explicit non-goal. Discovery is per-tenant only.

## 11b. Generalization to resources

The same pattern generalizes directly to resource discovery. A parallel
`resources:tenant-<id>` stream can publish `resource_published`-style events
for git repos, local paths, durable-stream blobs, S3 prefixes, Docker volumes,
OCI layers, and similar backing stores. The mechanism is identical: append
resource presence events, replay them into a projection, and treat the durable
stream as the universal discovery plane. A companion proposal,
`docs/proposals/resource-discovery.md`, specifies that side; this doc stays
focused on Hosts and runtimes.

## 12. Milestones

- **M1** — write this proposal doc
- **M2** — write `verification/spec/deployment_discovery.tla`
- **M3** — implement `StreamDeploymentPeerRegistry` in `fireline-tools`
- **M4** — wire Host self-publishing into `crates/fireline-host/src/bootstrap.rs`
  and the managed Host lifecycle
- **M5** — delete `LocalPeerDirectory`, `ControlPlanePeerRegistry`, and the old
  `GET /v1/runtimes` discovery read path
- **M6** — update `docs/proposals/fireline-host-cleanup-plan.md` §C4 / §C5 to
  reflect the deeper scope
- **M7** — wire the cross-Host migration demo into browser-harness

## Appendix: Why this is the right primitive boundary

`LocalPeerDirectory` was wrong because it wrote deployment truth to local disk.
`ControlPlanePeerRegistry` was wrong because it made the Tools primitive depend
on a Host-specific HTTP read path that already had a stream-derived replacement
waiting behind it. This proposal fixes both without inventing a new primitive:

- Host owns publishing its own presence
- Tools owns consuming discovery through `PeerRegistry`
- Durable Streams owns the shared substrate

That is the cleanest alignment with the primitive taxonomy already established
in [`./client-primitives.md`](./client-primitives.md) and the Host/Sandbox
taxonomy in [`./runtime-host-split.md`](./runtime-host-split.md).
