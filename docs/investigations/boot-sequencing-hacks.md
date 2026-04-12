# Boot Sequencing Hacks Audit

## TL;DR

The current boot path still carries multiple synchronization hacks from the old
architecture:

- readiness is inferred by polling local files and local HTTP endpoints
- durable streams are "ensured" with retry loops instead of being treated as a
  normal external dependency
- liveness still depends on a control-plane heartbeat/stale scanner model
- direct-host and managed-host startup still write last-known runtime state into
  a local registry file

Under the current target architecture in
`docs/proposals/deployment-and-remote-handoff.md` and
`docs/proposals/cross-host-discovery.md`, those are mostly vestigial. Fireline
now assumes one binary, an external durable-streams service, and stream-backed
Host/runtime discovery. The remaining sleeps, retry loops, and file-based
readiness checks are therefore not neutral glue. They are evidence that boot is
still partially designed around the previous embedded-stream-server /
separate-control-plane / TOML-registry world.

## Scope

Audited files:

- `crates/fireline-host/src/bootstrap.rs`
- `crates/fireline-harness/src/runtime_topology.rs`
- `crates/fireline-host/src/local_provider.rs`
- `crates/fireline-host/src/control_plane.rs`
- `src/main.rs`

Categories:

- `A` Arbitrary sleeps / delays
- `B` Retry / ensure loops for stream existence
- `C` File-based readiness detection
- `D` Temporal coupling / boot ordering

## Findings Summary

| Severity | Categories | File:line | Summary |
|---|---|---|---|
| `P0` | `A`, `C`, `D` | `crates/fireline-host/src/local_provider.rs:58-96` | Child readiness is still detected by polling `RuntimeRegistry` every 100ms until the child updates a file-backed descriptor. |
| `P1` | `A`, `B`, `D` | `crates/fireline-host/src/bootstrap.rs:180-182,345-360` | Boot retries stream creation with a 50ms sleep loop before proceeding. |
| `P1` | `A`, `B`, `D` | `crates/fireline-harness/src/runtime_topology.rs:147-155,372-387` | Topology boot "ensures" audit streams exist with the same retry loop. |
| `P1` | `A`, `D` | `src/main.rs:389,436,526-544` | Parent startup still polls `/healthz` with 50ms sleeps before registration/logging. |
| `P1` | `C`, `D` | `src/main.rs:464-503,508-512` | Managed and local fallback paths still persist runtime state into `RuntimeRegistry` on start/stop. |
| `P1` | `C`, `D` | `crates/fireline-host/src/control_plane.rs:71-111,130-191` | Control-plane liveness still depends on `HeartbeatTracker` + stale scanner + file-backed registry mutation. |
| `P2` | `C`, `D` | `crates/fireline-host/src/control_plane.rs:54-65` | `listen_addr_file` still writes the bound address to disk as process-to-process bootstrap IPC. |
| `P2` | `C`, `D` | `crates/fireline-host/src/local_provider.rs:118-159` | The child-launch contract still threads `--runtime-registry-path` and `--peer-directory-path` into the subprocess. |
| `P2` | `C`, `D` | `src/main.rs:373-385,419-433` | `peer_directory_path` is still carried through direct-host and managed-host boot even though discovery is now stream-backed. |

## Detailed Findings

### 1. `RuntimeRegistry` polling is still the child readiness gate

- File: `crates/fireline-host/src/local_provider.rs:58-96`
- Categories: `A`, `C`, `D`
- Severity: `P0`

The hack:

- `wait_for_runtime_ready()` loops until `startup_timeout`
- every pass, it checks `self.runtime_registry.get(host_key)?`
- if the descriptor is not yet `Ready`, it checks `child.try_wait()?`
- otherwise it sleeps `self.poll_interval`, currently `100ms`

Why it exists historically:

- This is the old separate-control-plane bootstrap contract: parent process
  spawns `fireline`, child process eventually updates `RuntimeRegistry`, parent
  notices the TOML row changed and treats that as readiness.
- That model made sense when the control plane owned readiness and child
  runtimes had no shared discovery plane.

Why it is wrong now:

- The new architecture's durable truth is the stream, not the local registry.
- The new discovery contract is `host_registered` / `runtime_provisioned` on
  `hosts:tenant-<tenant_id>`, not "TOML row reached `Ready`."
- If `RuntimeRegistry` is deleted, this launch path immediately loses its
  readiness signal.

Clean replacement:

- Either make child bootstrap return an explicit readiness signal to the parent
  over a direct in-process or subprocess channel, or
- make the parent wait for the child's stream-published `runtime_provisioned`
  / `host_registered` event instead of polling a local file
- remove the 100ms sleep loop entirely

### 2. `bootstrap.rs` retries stream creation with a 50ms sleep loop

- File: `crates/fireline-host/src/bootstrap.rs:180-182,345-360`
- Categories: `A`, `B`, `D`
- Severity: `P1`

The hack:

- boot calls `ensure_named_streams(...)`
- then calls `ensure_stream_exists(&state_stream_handle)` and
  `ensure_stream_exists(&host_stream_handle)`
- `ensure_stream_exists()` repeatedly calls `stream.create_with(...)`
- on failure it sleeps `50ms` and retries until a `5s` deadline

Why it exists historically:

- This looks like inherited startup tolerance from a world where the stream
  service might be embedded, just-started, or not immediately ready to accept
  create requests.
- It also reflects an older assumption that stream creation is a boot-time
  imperative rather than a property of the external durable-streams service.

Why it is wrong now:

- The deployment proposal treats durable-streams as an external, required,
  always-on dependency.
- The discovery proposal treats the stream as the primary substrate, not
  something Fireline should babysit into existence with retry sleeps.
- A loop here hides initialization ordering mistakes instead of naming them.

Clean replacement:

- Prefer producer-side auto-create on first append if the durable-streams API
  supports it, or
- fail once, fast, and clearly if the configured stream service is unavailable,
  rather than polling it into readiness
- if stream provisioning must remain explicit, do it declaratively outside the
  runtime boot path

### 3. `runtime_topology.rs` still "ensures" audit streams before boot

- File: `crates/fireline-harness/src/runtime_topology.rs:147-155,372-387`
- Categories: `A`, `B`, `D`
- Severity: `P1`

The hack:

- `ensure_named_streams()` parses the topology, extracts audit stream names,
  creates `DurableStream` handles, and calls `ensure_stream_exists()` on each
- `ensure_stream_exists()` is another `5s` retry loop with `sleep(50ms)`

Why it exists historically:

- Audit streams were likely treated as "special side streams" that had to be
  provisioned before any component started emitting trace data.
- That matches the older embedded infrastructure mindset: boot takes
  responsibility for making every side channel exist before traffic flows.

Why it is wrong now:

- Under the new architecture, these streams are just named durable-streams
  topics on an external service.
- The harness should not have to poll for them. That turns topology assembly
  into infra orchestration.
- It also duplicates the same stream-creation policy already present in
  `bootstrap.rs`.

Clean replacement:

- Let the `AuditTracer` producer create the stream on first append, or
- pre-provision named audit streams as deployment-time infrastructure
- remove the duplicated 50ms retry policy from the harness layer

### 4. Parent boot still polls `/healthz` before trusting the child

- File: `src/main.rs:389,436,526-544`
- Categories: `A`, `D`
- Severity: `P1`

The hack:

- both `run_direct_host()` and `run_managed_runtime()` call
  `wait_for_runtime_listener_ready(&handle.health_url).await?`
- `wait_for_runtime_listener_ready()` loops for up to `5s`
- it issues `GET /healthz`
- on failure it sleeps `50ms` and retries

Why it exists historically:

- This is another "give the other process time to start" safeguard.
- The caller does not trust `bootstrap::start()` to mean "the listener is
  ready," so it performs an extra out-of-band readiness probe.

Why it is wrong now:

- In the one-binary architecture, `bootstrap::start()` should have a precise
  contract about when it returns.
- If it returns before the listener is ready, that is the lifecycle bug.
- If it returns after readiness, the extra HTTP polling is redundant.

Clean replacement:

- Tighten `bootstrap::start()` so it returns only after the ACP/health listener
  is ready, or
- have bootstrap expose an explicit readiness signal instead of making the
  caller poll a local HTTP endpoint
- remove the hard-coded retry sleep

### 5. `RuntimeRegistry` is still written on managed/local fallback start-stop

- File: `src/main.rs:464-503,508-512`
- Categories: `C`, `D`
- Severity: `P1`

The hack:

- in push mode, the child registers with the control plane and starts a
  heartbeat loop
- in non-push mode, `run_managed_runtime()` writes `descriptor.clone()` into
  `RuntimeRegistry`
- on shutdown it reloads the registry and upserts a `Stopped` row
- `load_runtime_registry()` is still the normal fallback path

Why it exists historically:

- This is the legacy last-known-state mechanism from before stream-backed
  discovery and stream-backed lifecycle projection were available.
- It let local tooling recover "what runtimes exist" without a shared state
  stream.

Why it is wrong now:

- The discovery proposal explicitly makes the stream the deployment truth and
  deletes file-backed peer discovery.
- A local registry write on start/stop is therefore stale shadow state.
- It also means direct-host and non-push paths are still structurally different
  from the desired architecture.

Clean replacement:

- delete `RuntimeRegistry` start/stop writes
- publish lifecycle state solely through the stream-backed Host/runtime events
- if callers still need current status, they should read the stream-derived
  projection rather than a TOML file

### 6. Control-plane liveness still depends on stale scanning and heartbeat state

- File: `crates/fireline-host/src/control_plane.rs:71-111,130-191`
- Categories: `C`, `D`
- Severity: `P1`

The hack:

- boot loads `RuntimeRegistry`
- constructs `HeartbeatTracker`
- spawns `spawn_stale_runtime_task(...)`
- every scan interval, it asks for stale keys, re-reads the registry, marks
  ready runtimes as `Stale`, and forgets heartbeat entries for stopped/broken
  rows

Why it exists historically:

- This is the old separate control-plane liveness model: child processes POST
  heartbeats, the control plane owns freshness, and the registry row is the
  authoritative lifecycle surface.

Why it is wrong now:

- The current discovery design already introduces `host_heartbeat` as a stream
  event and explicitly says missing heartbeats should only affect reader-side
  freshness.
- The deployment model says the durable stream is the only stateful component.
- A stale scanner that mutates local registry rows is the exact opposite of
  that design.

Clean replacement:

- remove `HeartbeatTracker`, stale scanning, and registry mutation from the
  control plane
- treat heartbeats as stream-published liveness hints only
- derive freshness in the projection layer (`DeploymentIndex` /
  stream-backed host index), not via a side task mutating local state

### 7. `listen_addr_file` is still used as bootstrap IPC

- File: `crates/fireline-host/src/control_plane.rs:54-65`
- Categories: `C`, `D`
- Severity: `P2`

The hack:

- after binding, the control plane optionally writes `bound_addr` to
  `listen_addr_file`

Why it exists historically:

- This is classic file-based process handoff: a parent process, test harness,
  or shell script needs to discover which random port the child bound.

Why it is wrong now:

- It is another file-based side channel in a codebase that is otherwise moving
  to declarative startup and stream-backed discovery.
- It is not a correctness bug by itself, but it is stale lifecycle plumbing.

Clean replacement:

- return the bound address through the actual process orchestration layer
  instead of writing to disk, or
- keep port selection deterministic in environments that need discovery

### 8. The child launch contract still passes registry and peer-directory paths

- File: `crates/fireline-host/src/local_provider.rs:118-159`
- Categories: `C`, `D`
- Severity: `P2`

The hack:

- child process launch still passes `--runtime-registry-path`
- it optionally passes `--peer-directory-path`
- the child therefore still expects both local files to be meaningful parts of
  boot

Why it exists historically:

- These flags are leftovers from the old local bootstrap story:
  `RuntimeRegistry` for parent/child lifecycle state and `LocalPeerDirectory`
  for peer discovery.

Why it is wrong now:

- The discovery proposal explicitly deletes `LocalPeerDirectory`.
- The deployment and stream-as-truth direction makes `RuntimeRegistry` a stale
  shadow store.
- Keeping both flags in the subprocess contract preserves obsolete topology
  assumptions.

Clean replacement:

- remove both file-path flags from the child bootstrap contract
- pass only declarative stream/discovery configuration needed for
  self-publication onto durable streams

### 9. `peer_directory_path` is still threaded through host boot

- File: `src/main.rs:373-385,419-433`
- Categories: `C`, `D`
- Severity: `P2`

The hack:

- both direct-host and managed-host boot still compute a
  `peer_directory_path`
- that path is threaded into `BootstrapConfig` even though the current
  discovery direction is stream-backed

Why it exists historically:

- This is a lingering parameter from the earlier local mesh model where peer
  discovery meant "shared TOML file on disk."

Why it is wrong now:

- The durable-streams-backed discovery proposal explicitly rejects keeping
  `LocalPeerDirectory` as even a dev fallback.
- Carrying the path through boot is therefore pure stale configuration.

Clean replacement:

- remove `peer_directory_path` from CLI, bootstrap config, and provider
  assembly
- discovery should depend on the tenant stream and nothing else

## Recommended Cleanup Order

1. Replace child readiness detection first.
   The `RuntimeRegistry` polling loop in `local_provider.rs` is the deepest
   remaining architectural dependency on the old world.
2. Remove `wait_for_runtime_listener_ready()` by tightening
   `bootstrap::start()`'s contract.
3. Delete both `ensure_stream_exists()` retry loops.
   Once durable-streams is treated as a normal external dependency, stream
   creation should not be a sleep-based boot ritual in two places.
4. Delete `RuntimeRegistry`, `HeartbeatTracker`, stale scanning, and local
   start/stop upserts together.
   Those four pieces reinforce each other.
5. Strip the file-based bootstrap residue.
   `listen_addr_file`, `peer_directory_path`, and `--runtime-registry-path`
   should disappear once the lifecycle and discovery cuts above land.

## Bottom Line

The current boot code still assumes that readiness is something Fireline must
discover indirectly by sleeping, polling, and checking side stores. Under the
new architecture, readiness should be explicit and structural:

- durable-streams is an external prerequisite, not something to "wait into
  existence"
- Host/runtime discovery is stream-published, not file-persisted
- freshness is projection logic, not stale-scanner mutation
- `bootstrap::start()` should define readiness directly instead of forcing its
  callers to guess with retries

Most of these are not random cleanup nits. They are the last concrete places
where the old architecture is still shaping runtime behavior.
