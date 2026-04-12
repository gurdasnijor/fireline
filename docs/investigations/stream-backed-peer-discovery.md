# Stream-backed peer discovery

## TL;DR

- Long-term, peer discovery should be stream-backed: `ControlPlanePeerRegistry` is reading a control-plane HTTP view that still fronts `RuntimeRegistry`, so it is on the wrong side of the stream-as-truth cut.
- For **managed runtimes on a shared state stream**, `RuntimeIndex` already carries enough data to satisfy `PeerRegistry`; the only real join is `runtime_endpoints + runtime_spec.name`.
- This is **not** a total drop-in today: direct-host bootstrap still emits no `runtime_endpoints`, and direct-host runtimes do not share a common state stream, so `LocalPeerDirectory` remains a transitional fallback.

## Current state

- `ControlPlanePeerRegistry` (`crates/fireline-host/src/control_plane_peer_registry.rs`) does a `GET /v1/runtimes`, deserializes `RuntimeDescriptor`, filters to `Ready|Busy|Idle`, and maps that into `fireline_tools::directory::Peer`.
- `LocalPeerDirectory` (`crates/fireline-tools/src/peer/directory.rs`) is the file-backed fallback. Bootstrap uses it when `control_plane_url` is `None`, registers on startup, and unregisters on shutdown.
- Bootstrap wiring is binary: `crates/fireline-host/src/bootstrap.rs:141-153` picks `ControlPlanePeerRegistry` in push/control-plane mode and `LocalPeerDirectory` otherwise.
- Consumers are thin. `PeerComponent` injects the MCP server, and `crates/fireline-tools/src/peer/mcp_server.rs:100-138` only needs `list_peers()` / `lookup_peer()` returning `{ runtime_id, agent_name, acp_url, state_stream_url, registered_at_ms }`.

## The stream-as-truth concern

The current HTTP path is architecturally wrong, but only partially wrong in practice.

- Wrong architecturally: the handoff doc is explicit that the stream-as-truth refactor is only **mid-sequence**. `RuntimeIndex` exists, but production control plane step 2 is deferred, so `/v1/runtimes` still reads `RuntimeRegistry`, not the stream-derived projection.
- Not a proven functional bug today: the same handoff cites `tests/runtime_index_agreement.rs` as evidence that, for the control-plane-managed path, the stream projection and registry currently agree on live/stopped lifecycle. The divergence window is therefore bounded, not theoretical chaos.
- The named commits `0d20237`, `b5161f1`, and `c64c541` are crate moves (`runtime_identity` into `fireline-session`, providers into `fireline-sandbox`, `runtime_index` into `fireline-session`), not completion of the read-path flip. They did not make the HTTP adapter stream-backed.

So the verdict is: **the poll-over-HTTP pattern is temporary technical debt, not an urgent correctness bug**.

## Proposed StreamProjectedPeerRegistry

Type surface:

```rust
pub struct StreamProjectedPeerRegistry {
    runtime_index: RuntimeIndex,
}

impl PeerRegistry for StreamProjectedPeerRegistry {
    async fn list_peers(&self) -> Result<Vec<Peer>> {
        // runtime_index.list_endpoints().await
        // filter Ready | Busy | Idle
        // join runtime_key -> spec_for(runtime_key) to get create_spec.name
        // map to Peer
    }
}
```

What it should read:

- `RuntimeIndex::list_endpoints()` for `runtime_id`, `status`, `acp.url`, `state.url`, and timestamps.
- `RuntimeIndex::spec_for(runtime_key)` for the canonical runtime name (`create_spec.name`).

That is enough for the current `PeerRegistry` contract. The only fallback worth keeping is:

- if `spec_for(runtime_key)` is missing, derive `agent_name` from `runtime_id` the same way `ControlPlanePeerRegistry` does today.

Gaps:

- No field-level blocker for the **managed shared-stream path**.
- The real blocker is event/topology coverage:
  - direct-host bootstrap emits `runtime_spec` but **not** `runtime_endpoints` (`crates/fireline-host/src/bootstrap.rs:218-269`), so `RuntimeIndex` cannot reconstruct peer ACP/state endpoints there.
  - direct-host runtimes in tests like `tests/mesh_baseline.rs` each get their own auto-generated state stream, so a `RuntimeIndex` attached to one runtime would only see itself.

### Where it lives

Recommendation: **keep the impl in `fireline-host` for now**.

Rationale:

- `fireline-tools` owns the trait, but adding a `fireline-tools -> fireline-session` dependency would violate the current primitive split.
- `fireline-session` owns `RuntimeIndex`, but adding a `fireline-session -> fireline-tools` dependency would be the same problem in reverse.
- `fireline-host` already depends on both crates and is the existing bootstrap wiring layer, so this adapter is honest there as composition glue even if peer discovery is not a Host primitive.

Longer-term, if the project wants the impl to move out of host, the cleaner fix is a lower shared crate or a trait relocation, not a new cross-primitive dependency.

## Transition plan

- Managed/control-plane path first: once stream-as-truth step 2 lands and the host/control plane owns a real shared state-stream subscription, swap `ControlPlanePeerRegistry` for `StreamProjectedPeerRegistry` at the bootstrap wiring site.
- Keep consumer code unchanged: `PeerComponent`, `list_peers`, and `prompt_peer` should not care which `PeerRegistry` impl they receive.
- Test impact:
  - `tests/control_plane_docker.rs` is the main managed-path peer-discovery test today; it should keep passing under the new adapter because it already provisions runtimes onto one shared state stream.
  - `tests/mesh_baseline.rs` is the direct-host counterexample; it currently works only because `LocalPeerDirectory` bridges separate per-runtime streams.
- `LocalPeerDirectory` fate:
  - **not obsolete immediately**
  - obsolete only after direct-host mode also publishes `runtime_endpoints` into a common stream, or after direct-host peer discovery is explicitly scoped to "shared-stream mode only"

Net: this can cut over cleanly for managed runtimes, but **direct-host needs a follow-up lane** before the file-backed fallback can be deleted.

## Open questions for the user

- Is peer discovery supposed to mean "all runtimes on this Host's shared state stream" or "all runtimes reachable through the same durable-streams service"? `RuntimeIndex` only solves the first without another aggregation layer.
- Is `create_spec.name` the intended canonical peer name, or is deriving `agent_name` from `runtime_id` still acceptable as fallback behavior?
- Should direct-host mode keep peer discovery across separate per-runtime streams, or is it acceptable to require shared-stream mode for peer calls?
