//! Stream-as-truth agreement invariant test.
//!
//! Proves that the new `RuntimeIndex` projection — which derives
//! runtime existence from `runtime_spec` + `runtime_instance`
//! envelopes on the shared durable state stream — observes the same
//! runtime lifecycle that the in-memory `RuntimeRegistry` + control
//! plane HTTP API already observe today.
//!
//! This is the empirical half of the stream-as-truth refactor
//! (complementing the formal check in `crates/fireline-semantics/src/stream_truth.rs`
//! at commit `8cc07ed`). If this test stays green across all
//! existing control-plane flows, we know the stream already carries
//! enough information to replace `RuntimeRegistry` with a pure
//! stream-derived view — and commits C/D/E of the stream-as-truth
//! sequence can proceed.
//!
//! The current coverage is limited to the control-plane-managed
//! path (which emits both `runtime_spec` and `runtime_instance`).
//! Direct-host mode (`src/bootstrap.rs::start`) emits only
//! `runtime_instance` and not `runtime_spec`; see the known-gap
//! note in `src/runtime_index.rs`. Closing that gap is a prerequisite
//! for a fully stream-derived `RuntimeRegistry` replacement.

#![allow(clippy::uninlined_format_args)]

#[path = "support/managed_agent_suite.rs"]
mod managed_agent_suite;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use fireline_session::{RuntimeIndex, RuntimeInstanceStatus, RuntimeMaterializer, RuntimeStatus};
use managed_agent_suite::{ControlPlaneHarness, DEFAULT_TIMEOUT, ManagedAgentHarnessSpec};

/// Precondition: control plane is up with its shared state stream;
/// a managed runtime has been created through the usual
/// `POST /v1/runtimes` path.
///
/// Action: attach a fresh [`RuntimeIndex`] to the shared state
/// stream via [`RuntimeMaterializer::connect`], wait for the
/// preload to catch up to live edge, then inspect both the
/// control plane's `RuntimeRegistry` view (via the
/// [`ControlPlaneHarness::wait_for_status`] path the harness already
/// exposes) and the stream-derived `RuntimeIndex` view.
///
/// Invariant proven: **stream-as-truth agreement (control-plane path)**
/// — for a runtime created through the control plane, the stream
/// projection observes (a) a `runtime_spec` envelope keyed by
/// `runtime_key`, and (b) a `runtime_instance` envelope keyed by
/// `runtime_id` with `status == Running`. This matches the
/// registry's ready view.
#[tokio::test]
async fn runtime_index_agrees_with_registry_on_a_live_managed_runtime() -> Result<()> {
    let control_plane = ControlPlaneHarness::spawn(true).await?;

    let result = async {
        let spec =
            ManagedAgentHarnessSpec::new("runtime-index-agreement-live").with_testy_load_agent();
        let runtime = control_plane.create_runtime_from_spec(spec).await?;

        let index = Arc::new(RuntimeIndex::new());
        let materializer = RuntimeMaterializer::new(vec![index.clone()]);
        let task = materializer.connect(control_plane.shared_state_url());
        tokio::time::timeout(DEFAULT_TIMEOUT, task.preload())
            .await
            .context(
                "INVARIANT (stream-as-truth): RuntimeIndex preload must reach live edge \
                 within the default timeout so agreement assertions have complete state",
            )??;

        let spec_from_index = index.spec_for(&runtime.runtime_key).await;
        assert!(
            spec_from_index.is_some(),
            "INVARIANT (stream-as-truth): every control-plane-managed runtime must \
             have a `runtime_spec` envelope on the shared stream keyed by its \
             runtime_key (got None for runtime_key={})",
            runtime.runtime_key
        );
        assert_eq!(
            spec_from_index
                .as_ref()
                .map(|spec| spec.runtime_key.as_str()),
            Some(runtime.runtime_key.as_str()),
            "INVARIANT (stream-as-truth): runtime_spec.runtime_key must match the \
             envelope key the control plane wrote",
        );

        let running_ids = index
            .instance_ids_with_status(RuntimeInstanceStatus::Running)
            .await;
        assert!(
            running_ids.iter().any(|id| id == &runtime.runtime_id),
            "INVARIANT (stream-as-truth): runtime_instance.status=Running must be \
             observable for the control-plane-managed runtime_id on the shared \
             stream (looking for {}, saw {:?})",
            runtime.runtime_id,
            running_ids
        );

        // Step 1 of the registry-removal sequence: `runtime_endpoints`
        // envelopes let the stream projection reconstruct the full
        // RuntimeDescriptor (acp.url, state.url, helper_api_base_url,
        // status, timestamps) without reading through RuntimeRegistry.
        // This is the load-bearing agreement — once it holds for every
        // mutation point, the read path can flip to the projection.
        let endpoints_from_index =
            index
                .endpoints_for(&runtime.runtime_key)
                .await
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "INVARIANT (stream-as-truth): runtime_endpoints envelope must be \
                     observable for every control-plane-managed runtime (missing for {})",
                        runtime.runtime_key
                    )
                })?;
        assert_eq!(
            endpoints_from_index.runtime_key, runtime.runtime_key,
            "INVARIANT (stream-as-truth): runtime_endpoints.runtime_key must equal the \
             descriptor the control plane returned",
        );
        assert_eq!(
            endpoints_from_index.runtime_id, runtime.runtime_id,
            "INVARIANT (stream-as-truth): runtime_endpoints.runtime_id must agree with \
             the registry descriptor",
        );
        assert_eq!(
            endpoints_from_index.acp.url, runtime.acp.url,
            "INVARIANT (stream-as-truth): stream projection must carry the same \
             advertised acp URL the registry carries",
        );
        assert_eq!(
            endpoints_from_index.state.url, runtime.state.url,
            "INVARIANT (stream-as-truth): stream projection must carry the same \
             state stream URL the registry carries",
        );

        task.abort();
        Ok::<(), anyhow::Error>(())
    }
    .await;

    control_plane.shutdown().await;
    result
}

/// Precondition: control plane has been used to create a runtime
/// and then explicitly stop it via the same `POST /v1/runtimes/{key}/stop`
/// path that other harness tests exercise.
///
/// Action: drive the create → stop lifecycle against the control
/// plane; materialize a `RuntimeIndex` afterward and observe what
/// the stream says about the stopped runtime.
///
/// Invariant proven: **stream-as-truth observes stop transitions**
/// — the stream-derived projection sees a `runtime_instance` with
/// `status == Stopped` (or no Running entry) for a runtime that
/// the registry has transitioned to `RuntimeStatus::Stopped`. This
/// is the bound on divergence between the two sources during a
/// normal stop path.
///
/// Note: in the current wire shape there is no direct link from
/// `runtime_key` to `runtime_id` on the stream. The agreement
/// check here is symmetric on "no Running instance named `X`"
/// rather than "Stopped instance named `X`" because direct-host
/// instance events could temporarily lag behind registry state;
/// the invariant we need for stream-as-truth is that the stream
/// **eventually** agrees, not that it agrees under every interleaving.
#[tokio::test]
async fn runtime_index_observes_stopped_runtimes_on_the_shared_stream() -> Result<()> {
    let control_plane = ControlPlaneHarness::spawn(true).await?;

    let result = async {
        let spec =
            ManagedAgentHarnessSpec::new("runtime-index-agreement-stopped").with_testy_load_agent();
        let runtime = control_plane.create_runtime_from_spec(spec).await?;
        control_plane.stop_runtime(&runtime.runtime_key).await?;

        // Brief propagation window: stop()'s emit is synchronous+
        // flushed and every emit call now uses a fresh producer id
        // (see `emit_runtime_endpoints_persisted` in trace.rs), so we
        // just need enough time for the materializer's live-follow
        // loop to read the committed chunk.
        tokio::time::sleep(Duration::from_millis(500)).await;

        let index = Arc::new(RuntimeIndex::new());
        let materializer = RuntimeMaterializer::new(vec![index.clone()]);
        let task = materializer.connect(control_plane.shared_state_url());
        tokio::time::timeout(DEFAULT_TIMEOUT, task.preload())
            .await
            .context("INVARIANT (stream-as-truth): RuntimeIndex preload must reach live edge")??;

        // The runtime's spec should still be on the stream — specs
        // are monotonic inserts, not tombstoned on stop.
        assert!(
            index.spec_for(&runtime.runtime_key).await.is_some(),
            "INVARIANT (stream-as-truth): runtime_spec envelopes are not removed on \
             stop; the stopped runtime's spec must still be visible for post-mortem \
             inspection and resume",
        );

        // The Running set should not include this runtime_id.
        let running = index
            .instance_ids_with_status(RuntimeInstanceStatus::Running)
            .await;
        assert!(
            !running.iter().any(|id| id == &runtime.runtime_id),
            "INVARIANT (stream-as-truth): after a control-plane stop, the stopped \
             runtime_id must not appear in the Running projection (saw {:?})",
            running
        );

        // The endpoints projection should reflect the Stopped status
        // transition: after stop, the runtime_endpoints row for this
        // runtime_key must carry status=Stopped. This closes the loop
        // on stream-derived liveness: the stream alone tells you every
        // fact the registry knows about the stopped runtime.
        let endpoints = index
            .endpoints_for(&runtime.runtime_key)
            .await
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "INVARIANT (stream-as-truth): runtime_endpoints row must survive \
                     stop transitions for post-mortem inspection (missing for {})",
                    runtime.runtime_key
                )
            })?;
        assert!(
            matches!(endpoints.status, RuntimeStatus::Stopped),
            "INVARIANT (stream-as-truth): runtime_endpoints.status must reflect \
             the Stopped transition after a control-plane stop (got {:?})",
            endpoints.status
        );

        task.abort();
        Ok::<(), anyhow::Error>(())
    }
    .await;

    control_plane.shutdown().await;
    result
}
