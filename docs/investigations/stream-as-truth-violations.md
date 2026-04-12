# Stream-as-Truth Violations

## TL;DR

The remaining demo-blocking violations are concentrated in the Host/control-plane
stack:

- `RuntimeRegistry` is still a file-backed source of truth for host/runtime
  descriptors.
- child runtimes still report readiness and liveness via HTTP callbacks instead
  of durable-stream writes.
- stale detection still comes from an in-memory liveness map instead of stream
  replay.
- `RuntimeHost` still mutates local descriptor status first and only mirrors the
  result to the stream afterward.

That means Fireline still has a split-brain control path: the durable stream is
not yet the sole authority for lifecycle state.

## Scope

Searches run over `crates/`, `src/`, and `tests/support/`:

- file-backed state probes:
  `toml::from_str|toml::to_string|std::fs::write|std::fs::read_to_string|File::create|File::open`
- local-state filenames:
  `runtimes.toml|peers.toml|agents.toml|connections/`
- in-memory cache probes:
  `Mutex<HashMap|RwLock<HashMap|Arc<Mutex`
- status mutation probes:
  `\\.status = |status:.*Starting|status:.*Ready|status:.*Stale|status:.*Stopped`
- registration/liveness probes:
  `register|heartbeat|liveness|stale_`

Not counted as violations:

- local secrets/config reads such as
  `crates/fireline-harness/src/secrets.rs:341-354`
- temp fixture / test file writes
- listen-address files such as
  `crates/fireline-host/src/control_plane.rs:54-64`
- stream-backed projections such as `HostIndex`, `SessionIndex`,
  `ActiveTurnIndex`, and `StreamDeploymentPeerRegistry` even though they use
  `HashMap`/`RwLock`, because those maps are replay-derived views rather than
  side channels
- provider-local opaque handle maps that do not themselves drive observable
  state, such as `crates/fireline-sandbox/src/microsandbox.rs:105-118`

## Findings

| File:line | What it is doing now | What it should be doing | Severity |
|---|---|---|---|
| `crates/fireline-sandbox/src/registry.rs:18-115`<br>`crates/fireline-host/src/control_plane.rs:67-73`<br>`src/main.rs:485-503,508-512` | Persists live `HostDescriptor` state to local `runtimes.toml`, reloads it on startup, and writes `Ready`/`Stopped` updates from the managed-runtime fallback path. | Append lifecycle/spec/endpoints facts only to durable streams, then rebuild current host/runtime state from `fireline_session::HostIndex` instead of `RuntimeRegistry`. | P0 |
| `crates/fireline-sandbox/src/lib.rs:200-205`<br>`crates/fireline-host/src/router.rs:77-90` | Serves `GET /v1/runtimes` and `GET /v1/runtimes/{host_key}` from the local registry via `RuntimeHost::list/get`. | Serve status reads from the stream projection (`HostIndex::list_endpoints` / `HostIndex::endpoints_for`) so read authority matches replayed stream state. | P0 |
| `crates/fireline-sandbox/src/lib.rs:86-98,164-197,220-233,254-307,324-345` | Creates and mutates `HostDescriptor.status` (`Starting`, `Ready`, `Stale`, `Stopped`) in local memory / local registry first, then mirrors that state to the stream with `stream_trace`. | Make the durable stream append the authoritative write path; descriptors and status should be derived by projection, not mutated in a local record first. | P0 |
| `crates/fireline-host/src/router.rs:54-59,144-197`<br>`crates/fireline-host/src/control_plane_client.rs:34-118`<br>`src/main.rs:464-483` | Uses a child-to-parent HTTP callback protocol for `register` and `heartbeat`; the control plane treats that sideband callback as authority and updates local state from it. | Emit registration/liveness facts to durable streams and have the parent host observe them through stream replay/projection instead of HTTP callbacks. | P0 |
| `crates/fireline-sandbox/src/registry.rs:14,71-97`<br>`crates/fireline-host/src/heartbeat.rs:10-25`<br>`crates/fireline-host/src/control_plane.rs:130-191` | Maintains a separate in-memory liveness map and a timer-driven stale scan that marks descriptors `Stale` outside the stream. The liveness map is process-local and disappears on restart. | Represent heartbeats as stream events and derive freshness from replay timestamps in the projection, the same way cross-host discovery derives freshness from `host_heartbeat`. | P0 |
| `crates/fireline-host/src/local_provider.rs:57-97,118-139,184-194` | Passes `--runtime-registry-path` to child runtimes and polls `RuntimeRegistry` until the child flips to `Ready`. The registry file is the launch readiness oracle. | Wait for readiness from durable-stream-backed facts: either a `HostIndex` projection update or direct observation of the child's state stream / emitted endpoints. | P0 |
| `crates/fireline-sandbox/src/lib.rs:48-49,109-121,137-158,189-193,290-305` | Buffers `PersistedHostSpec` in `pending_host_specs` until a later `register(...)` call supplies enough information to flush the spec to the stream. A process crash can lose that buffered state. | Persist the requested host spec to the durable stream at provision time, then reconcile later endpoint/liveness updates through stream replay rather than an in-memory staging cache. | P1 |
| `crates/fireline-sandbox/src/lib.rs:48,128-132,210-216,240-246` | Uses `live_handles` as the local authority for whether a runtime is running and whether `stop` / `delete` can proceed. That map disappears on restart. | Keep handle maps as provider-internal plumbing only; authoritative runtime existence/status must come from stream projection, with provider reconciliation layered on top. | P1 |

## Severity Summary

- `P0`: 6 findings
- `P1`: 2 findings
- `P2`: 0 findings

## Conclusion

The cross-host discovery work made discovery stream-backed, but the Host
control-plane still is not. The remaining P0 work is the full removal of
`RuntimeRegistry`, the HTTP `register` / `heartbeat` callback loop, and the
timer-driven liveness side channel. Until those are gone, Fireline still has
two authorities for lifecycle state: durable streams and the local control-plane
process.
