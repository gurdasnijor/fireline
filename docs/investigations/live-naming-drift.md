# Live Naming Drift

This note records **live rename drift** that remains after the dead-code deletion pass. These items are not safe one-file deletions; they need a coordinated follow-up that updates the control-plane route names, test harnesses, and descriptor vocabulary together.

## Findings

### 1. `fireline-orchestration` still talks to `/v1/runtimes`

- [crates/fireline-orchestration/src/lib.rs](/Users/gnijor/gurdasnijor/fireline/crates/fireline-orchestration/src/lib.rs:57)
  `resume()` recreates a runtime by `POST`ing to `"{control_plane}/v1/runtimes"` and decodes a `HostDescriptor`.
- [crates/fireline-orchestration/src/lib.rs](/Users/gnijor/gurdasnijor/fireline/crates/fireline-orchestration/src/lib.rs:165)
  `lookup_runtime_for_session()` still fetches `GET /v1/runtimes/{host_key}`.
- [crates/fireline-orchestration/src/lib.rs](/Users/gnijor/gurdasnijor/fireline/crates/fireline-orchestration/src/lib.rs:190)
  `wait_for_runtime_ready()` still polls `GET /v1/runtimes/{host_key}`.

Why this is drift:
- The host router now exposes `/v1/sandboxes`, not `/v1/runtimes`.
- The crate still uses pre-rename `HostDescriptor` / `host_key` terminology throughout the control-plane path.

### 2. `bootstrap.rs` still imports old orchestration-owned names

- [crates/fireline-host/src/bootstrap.rs](/Users/gnijor/gurdasnijor/fireline/crates/fireline-host/src/bootstrap.rs:29)
  `bootstrap.rs` still imports `child_session_edge::ChildSessionEdgeWriter` and `load_coordinator::LoadCoordinatorComponent` from the `fireline_orchestration` crate namespace.

Why this is drift:
- The code is live, but the ownership/naming still reflects the earlier pre-collapse runtime/orchestration layout.
- Any rename here needs to move in lockstep with the orchestration crate and the managed-agent harness tests that depend on the same vocabulary.

### 3. `managed_agent_suite.rs` still exposes old runtime-oriented helper names and payloads

- [tests/support/managed_agent_suite.rs](/Users/gnijor/gurdasnijor/fireline/tests/support/managed_agent_suite.rs:272)
  `ControlPlaneHarness` still stores `runtime_registry_path` and `peer_directory_path`.
- [tests/support/managed_agent_suite.rs](/Users/gnijor/gurdasnijor/fireline/tests/support/managed_agent_suite.rs:317)
  `create_runtime_with_agent()` still posts to `/v1/runtimes`.
- [tests/support/managed_agent_suite.rs](/Users/gnijor/gurdasnijor/fireline/tests/support/managed_agent_suite.rs:326)
  The request body still sends deleted infrastructure fields: `provider`, `host`, `port`, `durableStreamsUrl`.
- [tests/support/managed_agent_suite.rs](/Users/gnijor/gurdasnijor/fireline/tests/support/managed_agent_suite.rs:345)
  `wait_for_status()` still polls `GET /v1/runtimes/{host_key}`.
- [tests/support/managed_agent_suite.rs](/Users/gnijor/gurdasnijor/fireline/tests/support/managed_agent_suite.rs:386)
  `stop_runtime()` still posts to `/v1/runtimes/{host_key}/stop`.
- [tests/support/managed_agent_suite.rs](/Users/gnijor/gurdasnijor/fireline/tests/support/managed_agent_suite.rs:399)
  `stop_all_live_runtimes()` still lists `/v1/runtimes`.

Why this is drift:
- The support harness remains the shared root for most managed-agent integration tests.
- Renaming it is a coordinated test-surface change, not dead-code deletion.

### 4. Managed-agent and control-plane tests still pass deleted CLI flags

- [tests/support/managed_agent_suite.rs](/Users/gnijor/gurdasnijor/fireline/tests/support/managed_agent_suite.rs:683)
  `spawn_control_plane()` still accepts `runtime_registry_path`, `peer_directory_path`, `heartbeat_scan_interval_ms`, and `stale_timeout_ms`.
- [tests/support/managed_agent_suite.rs](/Users/gnijor/gurdasnijor/fireline/tests/support/managed_agent_suite.rs:708)
  The spawned `fireline --control-plane` command still passes deleted flags:
  `--runtime-registry-path`, `--peer-directory-path`, `--heartbeat-scan-interval-ms`, `--stale-timeout-ms`.
- [tests/control_plane_push.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_push.rs:138)
  `spawn_control_plane()` still accepts and forwards the same deleted flags.
- [tests/control_plane_push.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_push.rs:159)
  The command still passes `--runtime-registry-path`, `--peer-directory-path`, `--heartbeat-scan-interval-ms`, and `--stale-timeout-ms`.
- [tests/control_plane_docker.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_docker.rs:437)
  Docker control-plane spawn still takes `runtime_registry_path` and `peer_directory_path`.
- [tests/control_plane_docker.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_docker.rs:457)
  The Docker path still passes deleted flags: `--runtime-registry-path`, `--peer-directory-path`.

Why this is drift:
- These tests are still written against the pre-collapse control-plane CLI contract.
- Updating them requires a coordinated rewrite of helper signatures and spawned command lines.

### 5. Tests still use `/v1/runtimes` request paths and runtime-oriented payloads

- [tests/control_plane_push.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_push.rs:57)
  Still creates via `POST /v1/runtimes`.
- [tests/control_plane_push.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_push.rs:58)
  The create payload still includes deleted fields: `provider`, `host`, `port`, `durableStreamsUrl`.
- [tests/control_plane_push.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_push.rs:83)
  Stop still posts to `/v1/runtimes/{host_key}/stop`.
- [tests/control_plane_push.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_push.rs:94)
  Delete still calls `/v1/runtimes/{host_key}`.
- [tests/control_plane_docker.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_docker.rs:130)
  List still uses `GET /v1/runtimes`.
- [tests/control_plane_docker.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_docker.rs:273)
  `create_runtime()` still posts to `/v1/runtimes`.
- [tests/control_plane_docker.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_docker.rs:283)
  The create payload still includes deleted fields: `host`, `port`, `durableStreamsUrl`.
- [tests/control_plane_docker.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_docker.rs:312)
  Wait/poll still uses `GET /v1/runtimes/{host_key}`.
- [tests/control_plane_docker.rs](/Users/gnijor/gurdasnijor/fireline/tests/control_plane_docker.rs:513)
  Cleanup still deletes `/v1/runtimes/{host_key}`.

Why this is drift:
- These tests are still asserting against the old route family and descriptor shape.
- They need a rename pass alongside `managed_agent_suite.rs` and `fireline-orchestration`, not one-off edits.

## Recommendation

Do not fix these piecemeal.

The remaining drift is no longer dead code. It is a coordinated rename pass across:

- `crates/fireline-orchestration`
- `crates/fireline-host/src/bootstrap.rs`
- `tests/support/managed_agent_suite.rs`
- control-plane integration tests

That pass should rename the route family, helper names, request bodies, and descriptor vocabulary together so the managed-agent test harness and orchestration crate do not diverge mid-sequence.
