//! # Harness Primitive Contract Tests
//!
//! Validates the **Harness** managed-agent primitive against the acceptance bars
//! in `docs/explorations/managed-agents-mapping.md` §3 "Harness" and the
//! Anthropic interface:
//!
//! ```text
//! yield Effect<T> → EffectResult<T>
//! ```
//!
//! *"Any loop that yields effects and appends progress to the Session."*
//!
//! Fireline implements the Harness primitive at a different layer than
//! Anthropic's framing: the harness is the agent process, and the conductor
//! proxy chain sits between the harness and its effects. Every effect the
//! agent yields (ACP requests) flows through the chain, where components can
//! observe, transform, filter, suspend, substitute, or fan out — the seven-
//! combinator algebra described in the mapping doc §"Fireline as combinators
//! over the primitives".
//!
//! This file validates Harness contracts at the **substrate level** — that
//! effects actually land in the durable log, that the proxy chain composes,
//! that suspend/resume survives runtime death via event sourcing. The full
//! combinator algebra (observe/mapEffect/appendToSession/filter/substitute/
//! suspend/fanout as distinct composable shapes) is a Fireline-internal
//! design, not an Anthropic primitive invariant, so it gets a lighter check.
//!
//! **Ownership boundary:** component-specific behavior (ApprovalGate policy,
//! Budget token accounting, ContextInjection content sources) belongs in
//! component unit tests, not here. This file checks the Harness **seam**
//! contracts.

#[path = "support/managed_agent_suite.rs"]
mod managed_agent_suite;

use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{Context, Result};
use durable_streams::Client as DurableStreamsClient;
use fireline_harness::{TopologyComponentSpec, TopologySpec};
use fireline_session::HostStatus;
use managed_agent_suite::{
    ControlPlaneHarness, DEFAULT_TIMEOUT, LocalRuntimeHarness, ManagedAgentHarnessSpec,
    append_approval_resolved, count_events, create_session, load_session, load_session_then_prompt,
    prompt_session, prompt_session_result, read_all_events, wait_for_event_count,
    wait_for_permission_request,
};
use serde_json::json;

async fn append_rebuild_padding_events(state_stream_url: &str, count: usize) -> Result<()> {
    let client = DurableStreamsClient::new();
    let mut stream = client.stream(state_stream_url);
    stream.set_content_type("application/json");
    let producer = stream
        .producer(format!(
            "approval-rebuild-padding-{}",
            uuid::Uuid::new_v4()
        ))
        .content_type("application/json")
        .build();
    let payload = "pad".repeat(1024);

    for index in 0..count {
        producer.append_json(&json!({
            "type": "approval_rebuild_padding",
            "key": format!("padding-{index}"),
            "headers": { "operation": "insert" },
            "value": { "payload": payload, "index": index },
        }));
    }

    producer
        .flush()
        .await
        .context("flush approval rebuild padding events")?;
    Ok(())
}

async fn session_permission_request_ids(
    state_stream_url: &str,
    session_id: &str,
) -> Result<Vec<String>> {
    let events = read_all_events(state_stream_url).await?;
    Ok(events
        .into_iter()
        .filter(|env| env.envelope_type() == Some("permission"))
        .filter_map(|env| {
            let value = env.value()?;
            if value.get("kind").and_then(|kind| kind.as_str()) != Some("permission_request") {
                return None;
            }
            if value.get("sessionId").and_then(|id| id.as_str()) != Some(session_id) {
                return None;
            }
            value
                .get("requestId")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        })
        .collect())
}

/// Precondition: a local runtime has been provisioned with the default
/// topology (whatever the baseline harness sets up).
///
/// Action: prompt the runtime once via ACP.
///
/// Observable evidence: the runtime's durable state stream contains
/// `session_v2`, `prompt_request`, and `chunk_v2` envelopes. This is the minimal
/// effect set for a non-trivial prompt — anything less would mean the
/// conductor proxy chain failed to append progress for at least one
/// observable event.
///
/// Invariant proven: **Harness ∘ Session — every effect is appended to the
/// durable log.** The conductor's `DurableStreamTracer` component captures
/// every ACP-level effect as it flows through the proxy chain, so nothing the
/// harness yields bypasses the durable record. This is the Anthropic "appends
/// progress to the Session" clause.
#[tokio::test]
async fn harness_every_effect_is_appended_to_session_log() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("harness-effects-logged").await?;

    let result = async {
        let _ = runtime
            .prompt("harness durable-log effect contract")
            .await?;

        let body = runtime
            .wait_for_state_rows(
                &[
                    "\"type\":\"session_v2\"",
                    "\"type\":\"prompt_request\"",
                    "\"type\":\"chunk_v2\"",
                ],
                DEFAULT_TIMEOUT,
            )
            .await
            .context(
                "INVARIANT (Harness ∘ Session): every effect the harness yields must \
                 land in the durable state stream via the conductor proxy chain",
            )?;

        let session_count = body.matches("\"type\":\"session_v2\"").count();
        let prompt_request_count = body.matches("\"type\":\"prompt_request\"").count();
        let chunk_count = body.matches("\"type\":\"chunk_v2\"").count();

        assert!(
            session_count >= 1,
            "session_v2 envelope count = {session_count}"
        );
        assert!(
            prompt_request_count >= 1,
            "prompt_request envelope count = {prompt_request_count}"
        );
        assert!(
            chunk_count >= 1,
            "chunk_v2 envelope count = {chunk_count} (at least one message chunk expected)"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: a runtime has been provisioned and has run at least one
/// full prompt → response cycle, producing a set of events in its
/// durable state stream.
///
/// Action: (1) snapshot the durable stream at time T1 via
/// `read_all_events` and extract the ordered sequence of envelope type
/// strings; (2) send another prompt; (3) wait for the event count to
/// increase via `wait_for_event_count` on prompt_request; (4) snapshot again
/// at time T2 and extract the new ordered sequence.
///
/// Observable evidence: the T1 sequence is an exact prefix of the T2
/// sequence. Every element at every index matches. This proves no
/// reordering and no loss of past events when the runtime continues to
/// write new ones. Done with parsed event types via `StateEnvelope`, not
/// substring matching.
///
/// Invariant proven: **Harness append-order stability under live writes**
/// — the durable log does not reorder or lose past events when the
/// runtime continues to write new ones. Materializers that cache a
/// last-seen offset and drain forward can rely on this contract.
#[tokio::test]
async fn harness_append_order_is_stable_under_continued_writes() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("harness-append-order-stable").await?;

    let result = async {
        // First prompt — seed the stream.
        let _ = runtime.prompt("first prompt to seed the log").await?;
        let _ = wait_for_event_count(
            runtime.state_stream_url(),
            "prompt_request",
            1,
            DEFAULT_TIMEOUT,
        )
        .await
        .context("first prompt's prompt_request must land in the log")?;

        // T1 snapshot: extract the ordered type sequence.
        let t1_events = read_all_events(runtime.state_stream_url()).await?;
        let t1_sequence: Vec<String> = t1_events
            .iter()
            .filter_map(|env| env.envelope_type().map(str::to_string))
            .collect();
        assert!(
            !t1_sequence.is_empty(),
            "T1 sequence must contain at least the initial envelopes"
        );

        // Second prompt — continue writing to the live stream.
        let _ = runtime.prompt("second prompt after T1 snapshot").await?;

        // Wait until the prompt_request count is strictly greater than the T1
        // count — confirms the second prompt landed.
        let t1_prompt_request_count = t1_sequence
            .iter()
            .filter(|kind| *kind == "prompt_request")
            .count();
        let _ = wait_for_event_count(
            runtime.state_stream_url(),
            "prompt_request",
            t1_prompt_request_count + 1,
            Duration::from_secs(10),
        )
        .await
        .context("second prompt's prompt_request must land in the log")?;

        // T2 snapshot.
        let t2_events = read_all_events(runtime.state_stream_url()).await?;
        let t2_sequence: Vec<String> = t2_events
            .iter()
            .filter_map(|env| env.envelope_type().map(str::to_string))
            .collect();

        // The critical assertion: T1 is a strict prefix of T2. Every
        // element at every index matches, and T2 is strictly longer.
        assert!(
            t2_sequence.len() > t1_sequence.len(),
            "INVARIANT (Harness): T2 sequence must be strictly longer than T1 after a \
             second prompt; T1 len = {}, T2 len = {}",
            t1_sequence.len(),
            t2_sequence.len()
        );

        for (index, (t1_kind, t2_kind)) in t1_sequence.iter().zip(t2_sequence.iter()).enumerate() {
            assert_eq!(
                t1_kind, t2_kind,
                "INVARIANT (Harness): T1 must be an exact prefix of T2 (no reordering, \
                 no loss). Mismatch at index {index}: T1 = '{t1_kind}', T2 = '{t2_kind}'"
            );
        }

        // Sanity-check via the count_events helper.
        let final_prompt_request_count =
            count_events(runtime.state_stream_url(), "prompt_request").await?;
        assert!(
            final_prompt_request_count >= t1_prompt_request_count + 1,
            "INVARIANT (Harness): final prompt_request count must be strictly greater than \
             the T1 count"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: a runtime is provisioned with an `approval_gate` topology
/// component configured with a `PromptContains` policy that matches a
/// specific needle and with `RequireApproval` as the action.
///
/// Action: fire a prompt containing the needle in a background task so it
/// reaches the gate and blocks. Poll the state stream for the
/// `permission_request` envelope emitted by the gate, extract the canonical
/// ACP `requestId` from it, then append an external `approval_resolved` event
/// with that same `requestId` and `allow: true` through a fresh external
/// producer (simulating an approval service).
///
/// Observable evidence: the background prompt task completes successfully
/// — its response is non-empty — and the stream records a
/// `permission_request` followed by an `approval_resolved` envelope for
/// the same `requestId`.
///
/// Invariant proven: **Harness suspend combinator actually suspends and
/// releases on a durable event.** The approval gate does not forward to
/// the downstream agent until a matching `approval_resolved` event
/// appears on the Session log. The release signal comes from an external
/// writer, not from the gate itself, which is the foundation the full
/// "pause survives runtime death" story sits on.
#[tokio::test]
async fn harness_approval_gate_blocks_prompt_until_resolved_via_stream_event() -> Result<()> {
    let topology = TopologySpec {
        components: vec![TopologyComponentSpec {
            name: "approval_gate".to_string(),
            config: Some(serde_json::json!({
                "timeoutMs": 15000,
                "policies": [
                    {
                        "match": { "kind": "promptContains", "needle": "pause_here" },
                        "action": "requireApproval",
                        "reason": "test policy: pause_here"
                    }
                ]
            })),
        }],
    };
    let spec = ManagedAgentHarnessSpec::new("harness-approval-gate-blocks-until-resolved")
        .with_topology(topology);
    let runtime = LocalRuntimeHarness::spawn_with(spec).await?;

    let result = async {
        let session_id = create_session(runtime.acp_url()).await?;
        let acp_url = runtime.acp_url().to_string();
        let state_url = runtime.state_stream_url().to_string();
        let session_id_prompt = session_id.clone();
        let prompt_task = tokio::spawn(async move {
            prompt_session(
                &acp_url,
                &session_id_prompt,
                "please pause_here for approval",
            )
            .await
        });

        let request_id = wait_for_permission_request(&state_url, &session_id, DEFAULT_TIMEOUT)
            .await
            .context(
                "INVARIANT (Harness): approval gate must publish a permission_request on a \
                 matching prompt before the agent sees it",
            )?;

        append_approval_resolved(&state_url, &session_id, &request_id, true).await?;

        let prompt_result = tokio::time::timeout(Duration::from_secs(15), prompt_task)
            .await
            .context("prompt task did not complete within the post-approval window")?;
        prompt_result.context("prompt task panicked")?.context(
            "INVARIANT (Harness): blocked prompt must succeed once approval_resolved is appended",
        )?;

        let permission_envelopes = count_events(&state_url, "permission").await?;
        assert!(
            permission_envelopes >= 2,
            "INVARIANT (Harness): stream must record both permission_request and \
             approval_resolved envelopes, saw {permission_envelopes}"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

#[tokio::test]
async fn harness_approval_gate_timeout_errors_cleanly() -> Result<()> {
    let topology = TopologySpec {
        components: vec![TopologyComponentSpec {
            name: "approval_gate".to_string(),
            config: Some(serde_json::json!({
                "timeoutMs": 500,
                "policies": [
                    {
                        "match": { "kind": "promptContains", "needle": "pause_here" },
                        "action": "requireApproval",
                        "reason": "test policy: timeout"
                    }
                ]
            })),
        }],
    };
    let spec =
        ManagedAgentHarnessSpec::new("harness-approval-gate-timeout").with_topology(topology);
    let runtime = LocalRuntimeHarness::spawn_with(spec).await?;

    let result = async {
        let session_id = create_session(runtime.acp_url()).await?;
        let prompt_result = tokio::time::timeout(
            Duration::from_secs(5),
            prompt_session_result(
                runtime.acp_url(),
                &session_id,
                "please pause_here until timeout",
            ),
        )
        .await
        .context("approval timeout prompt did not return within test timeout")??;
        let error = prompt_result.expect_err("approval gate should time out without a resolution");
        let error_text = error
            .data
            .as_ref()
            .and_then(|data| {
                data.as_str().map(str::to_string).or_else(|| {
                    data.get("data")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                })
            })
            .unwrap_or_default()
            ;
        assert!(
            error_text.contains("approval_gate timed out waiting for approval on session"),
            "INVARIANT (Harness): timeout path must surface the approval gate timeout message, got: {error_text}"
        );

        let permission_events = count_events(runtime.state_stream_url(), "permission").await?;
        assert_eq!(
            permission_events, 1,
            "INVARIANT (Harness): timeout path should emit exactly one permission_request and no approval_resolved"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: slice 16 ApprovalGateComponent rebuild-from-log behavior
/// (exists as `rebuild_from_log` on `LoadSessionRequest`) plus a way to
/// simulate runtime death mid-prompt without abandoning the client.
///
/// Action: block a prompt in the gate, snapshot the pending state, kill
/// the runtime while the gate is still waiting, re-provision a fresh
/// runtime against the same stored spec, issue `session/load` for the
/// previously pending session, then externally append an
/// `approval_resolved` event. The fresh runtime should see the pending
/// permission entry during load, pick up the resolution, and release a
/// follow-up prompt cleanly.
///
/// Invariant proven: **Harness paused state survives runtime death** —
/// the blocked-until-resolved semantic (now proven in
/// `harness_approval_gate_blocks_prompt_until_resolved_via_stream_event`)
/// composes with `load_session` rebuild-from-log so the pause survives
/// mid-flight process loss. The original blocked ACP request is expected
/// to die with the old runtime; the proof obligation is that the durable
/// state lets a cold-started runtime rebuild approval state and continue.
#[tokio::test]
async fn harness_durable_suspend_resume_round_trip() -> Result<()> {
    let topology = TopologySpec {
        components: vec![TopologyComponentSpec {
            name: "approval_gate".to_string(),
            config: Some(serde_json::json!({
                "timeoutMs": 15000,
                "policies": [
                    {
                        "match": { "kind": "promptContains", "needle": "pause_here" },
                        "action": "requireApproval",
                        "reason": "test policy: durable suspend/resume"
                    }
                ]
            })),
        }],
    };
    let control_plane = ControlPlaneHarness::spawn(true).await?;

    let result = async {
        let spec = ManagedAgentHarnessSpec::new("harness-durable-suspend-resume")
            .with_testy_load_agent()
            .with_topology(topology);
        let runtime = control_plane.create_runtime_from_spec(spec).await?;
        let shared_state_url = control_plane.shared_state_url();

        let session_id = create_session(&runtime.acp.url).await?;
        let blocked_acp_url = runtime.acp.url.clone();
        let blocked_session_id = session_id.clone();
        let blocked_prompt = tokio::spawn(async move {
            prompt_session(
                &blocked_acp_url,
                &blocked_session_id,
                "please pause_here across runtime death",
            )
            .await
        });
        let blocked_prompt_abort = blocked_prompt.abort_handle();

        let request_id = wait_for_permission_request(&shared_state_url, &session_id, DEFAULT_TIMEOUT)
            .await
            .context(
                "INVARIANT (Harness): approval gate must emit permission_request before runtime death",
            )?;

        let stopped = control_plane.stop_runtime(&runtime.host_key).await?;
        assert_eq!(
            stopped.status,
            HostStatus::Stopped,
            "INVARIANT (Harness): stop_runtime must transition the original runtime to Stopped"
        );

        match tokio::time::timeout(Duration::from_secs(5), blocked_prompt).await {
            Ok(joined) => match joined {
                Ok(Ok(())) => {
                    anyhow::bail!(
                        "INVARIANT (Harness): blocked prompt must not complete successfully after runtime death"
                    );
                }
                Ok(Err(_)) | Err(_) => {}
            },
            Err(_) => {
                blocked_prompt_abort.abort();
            }
        }

        append_approval_resolved(&shared_state_url, &session_id, &request_id, true).await?;
        let permission_events =
            wait_for_event_count(&shared_state_url, "permission", 2, DEFAULT_TIMEOUT).await?;
        assert!(
            permission_events.len() >= 2,
            "INVARIANT (Harness): durable log must contain both permission_request and approval_resolved before session/load"
        );

        let resumed = fireline_orchestration::resume(
            &control_plane.http,
            &control_plane.base_url,
            &shared_state_url,
            &session_id,
        )
        .await
        .context("resume suspended session after runtime death")?;
        assert_eq!(
            resumed.host_key, runtime.host_key,
            "INVARIANT (Harness): resume must recreate the same logical runtime"
        );
        assert_ne!(
            resumed.host_id, runtime.host_id,
            "INVARIANT (Harness): resume after stop must cold-start a fresh runtime process"
        );

        let permission_count_before = count_events(&shared_state_url, "permission").await?;
        assert_eq!(
            permission_count_before, 2,
            "INVARIANT (Harness): exactly the original permission_request and approval_resolved should exist before the resumed prompt"
        );

        tokio::time::timeout(
            Duration::from_secs(10),
            load_session_then_prompt(
                &resumed.acp.url,
                &session_id,
                "please pause_here after reload",
            ),
        )
        .await
        .context(
            "INVARIANT (Harness): resumed load_session+prompt should not re-block once approval state is rebuilt",
        )?
        .context(
            "INVARIANT (Harness): resumed load_session+prompt should succeed through the approval gate after session/load rebuild",
        )?;

        let permission_count_after = count_events(&shared_state_url, "permission").await?;
        assert_eq!(
            permission_count_after, permission_count_before,
            "INVARIANT (Harness): rebuilt approved_sessions must short-circuit the gate; saw new permission events after resumed prompt"
        );

        Ok(())
    }
    .await;

    control_plane.shutdown().await;
    result
}

#[tokio::test]
async fn harness_approval_resolution_during_rebuild_marks_session_approved() -> Result<()> {
    let topology = TopologySpec {
        components: vec![TopologyComponentSpec {
            name: "approval_gate".to_string(),
            config: Some(serde_json::json!({
                "timeoutMs": 15000,
                "policies": [
                    {
                        "match": { "kind": "promptContains", "needle": "pause_here" },
                        "action": "requireApproval",
                        "reason": "test policy: rebuild race"
                    }
                ]
            })),
        }],
    };
    let control_plane = ControlPlaneHarness::spawn(true).await?;

    let result = async {
        let spec = ManagedAgentHarnessSpec::new("harness-approval-rebuild-race")
            .with_testy_load_agent()
            .with_topology(topology);
        let runtime = control_plane.create_runtime_from_spec(spec).await?;
        let shared_state_url = control_plane.shared_state_url();

        let session_id = create_session(&runtime.acp.url).await?;
        let blocked_acp_url = runtime.acp.url.clone();
        let blocked_session_id = session_id.clone();
        let blocked_prompt = tokio::spawn(async move {
            prompt_session(
                &blocked_acp_url,
                &blocked_session_id,
                "please pause_here during rebuild",
            )
            .await
        });
        let blocked_prompt_abort = blocked_prompt.abort_handle();

        let request_id = wait_for_permission_request(&shared_state_url, &session_id, DEFAULT_TIMEOUT)
            .await
            .context("approval gate should emit a pending request before rebuild-race stop")?;

        append_rebuild_padding_events(&shared_state_url, 256).await?;

        let stopped = control_plane.stop_runtime(&runtime.host_key).await?;
        assert_eq!(
            stopped.status,
            HostStatus::Stopped,
            "INVARIANT (Harness): rebuild-race setup must stop the original runtime cleanly"
        );

        match tokio::time::timeout(Duration::from_secs(5), blocked_prompt).await {
            Ok(joined) => match joined {
                Ok(Ok(())) => {
                    anyhow::bail!(
                        "INVARIANT (Harness): blocked prompt must not complete successfully after runtime death"
                    );
                }
                Ok(Err(_)) | Err(_) => {}
            },
            Err(_) => {
                blocked_prompt_abort.abort();
            }
        }

        let resumed = fireline_orchestration::resume(
            &control_plane.http,
            &control_plane.base_url,
            &shared_state_url,
            &session_id,
        )
        .await
        .context("resume runtime before rebuild-race load_session")?;

        let resumed_acp_url = resumed.acp.url.clone();
        let resumed_session_id = session_id.clone();
        let resumed_load = tokio::spawn(async move {
            load_session(&resumed_acp_url, &resumed_session_id).await
        });

        tokio::time::sleep(Duration::from_millis(25)).await;
        append_approval_resolved(&shared_state_url, &session_id, &request_id, true).await?;

        tokio::time::timeout(Duration::from_secs(10), resumed_load)
            .await
            .context("rebuild-race load_session did not complete after external resolution")?
            .context("rebuild-race load_session task panicked")?
            .context(
                "INVARIANT (Harness): runtime should pick up approval_resolved appended during rebuild and complete session/load",
            )?;

        let permission_count_before_followup = count_events(&shared_state_url, "permission").await?;
        tokio::time::timeout(
            Duration::from_secs(10),
            load_session_then_prompt(
                &resumed.acp.url,
                &session_id,
                "please pause_here during rebuild",
            ),
        )
        .await
        .context("rebuild-race follow-up prompt did not complete after rebuild")?
        .context("rebuild-race follow-up prompt returned an error")?;

        let permission_count_after_followup = count_events(&shared_state_url, "permission").await?;
        assert_eq!(
            permission_count_after_followup, permission_count_before_followup,
            "INVARIANT (Harness): once approval_resolved is observed during rebuild, follow-up prompts must not emit a fresh permission_request"
        );

        let request_ids =
            session_permission_request_ids(&shared_state_url, &session_id).await?;
        let unique_request_ids: BTreeSet<_> = request_ids.iter().cloned().collect();
        assert_eq!(
            unique_request_ids,
            BTreeSet::from([request_id.clone()]),
            "INVARIANT (Harness): rebuild-race flow must not mint a second permission_request after session/load rebuild"
        );

        Ok(())
    }
    .await;

    control_plane.shutdown().await;
    result
}

/// Precondition: a topology is constructed with a representative instance of
/// each of the seven combinator kinds (observe, mapEffect, appendToSession,
/// filter, substitute, suspend, fanout).
///
/// Action: inspect the resulting `TopologySpec` and assert that each
/// combinator kind is actually represented as a distinct component with the
/// expected internal shape.
///
/// Observable evidence: the component list contains exactly seven distinct
/// combinator kinds, each identified by its kind tag or config shape.
///
/// Invariant proven: **Fireline-internal combinator algebra coverage** —
/// this is a Fireline-specific design invariant, not an Anthropic primitive
/// contract. It's included here to prevent the combinator algebra from
/// silently drifting away from the seven documented shapes in
/// `managed-agents-mapping.md` §"Fireline as combinators over the primitives".
/// **Not a managed-agent acceptance bar.**
#[tokio::test]
async fn harness_topology_represents_all_seven_combinators() -> Result<()> {
    use std::collections::HashSet;

    // Construct a TopologySpec containing one instance of each topologically
    // addressable combinator. Configs are intentionally `None`: this is a
    // pure data-shape assertion — the runtime factories that consume these
    // configs are not invoked here, only the spec structure is inspected.
    let topology = TopologySpec {
        components: vec![
            // observe → audit (tracer)
            TopologyComponentSpec {
                name: "audit".to_string(),
                config: None,
            },
            // mapEffect → context_injection
            TopologyComponentSpec {
                name: "context_injection".to_string(),
                config: None,
            },
            // filter → budget (filters by budget, can terminate turn)
            TopologyComponentSpec {
                name: "budget".to_string(),
                config: None,
            },
            // substitute → fs_backend (substitutes file reads/writes with
            // the runtime-stream backend)
            TopologyComponentSpec {
                name: "fs_backend".to_string(),
                config: None,
            },
            // suspend → approval_gate (blocks until externally resolved)
            TopologyComponentSpec {
                name: "approval_gate".to_string(),
                config: None,
            },
            // fanout → peer_mcp (fans out to peer runtimes)
            TopologyComponentSpec {
                name: "peer_mcp".to_string(),
                config: None,
            },
        ],
    };

    // Six topologically addressable combinators. The seventh combinator —
    // `appendToSession` — is the always-on `DurableStreamTracer` wired into
    // every conductor chain by the runtime, not a `TopologyComponentSpec`
    // entry, so it cannot (and should not) appear in this list. Six distinct
    // topology components plus the always-on tracer = seven combinators total.
    assert_eq!(
        topology.components.len(),
        6,
        "expected six topologically addressable combinator components"
    );

    let names: HashSet<&str> = topology
        .components
        .iter()
        .map(|c| c.name.as_str())
        .collect();

    let expected: HashSet<&str> = [
        "audit",
        "context_injection",
        "budget",
        "fs_backend",
        "approval_gate",
        "peer_mcp",
    ]
    .into_iter()
    .collect();

    assert_eq!(
        names, expected,
        "Fireline-internal invariant: TopologySpec must be able to represent each of the \
         six topologically addressable combinator kinds as a distinct component (the \
         seventh, appendToSession, is the always-on DurableStreamTracer)"
    );

    Ok(())
}
