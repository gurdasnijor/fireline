use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, LiveMode, Offset, Producer};
use fireline_harness::{
    awakeable_waiting_envelope, timer_fired_envelope, wake_timer_request_envelope,
    ActiveSubscriber, AwakeableResolver, CompletionKey, HandlerOutcome, StreamEnvelope,
    TraceContext, WakeTimerHandleError, WakeTimerRequest, WakeTimerRuntime, WakeTimerSubscriber,
    WorkflowContext, AWAKEABLE_RESOLVED_KIND, AWAKEABLE_WAITING_KIND, TIMER_FIRED_KIND,
    WAKE_TIMER_REQUESTED_KIND,
};
use sacp::schema::{RequestId, SessionId};
use serde_json::{json, Value};
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;
#[path = "support/wake_timer_runtime.rs"]
mod wake_timer_runtime;

use wake_timer_runtime::{ControlledWakeTimerRuntime, RecordingWakeTimerRuntime};

enum CompositeRaceOutcome<T> {
    Resolved(T),
    TimedOut(fireline_harness::TimerFired),
}

#[tokio::test]
async fn early_resolve_wins_prompt_wait_and_cancels_pending_timer_branch() -> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("ds-dp-composite-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;

    let session_id = SessionId::from("session-early-resolve");
    let request_id = RequestId::from("request-early-resolve".to_string());
    let key = CompletionKey::prompt(session_id.clone(), request_id.clone());
    let trace_context = TraceContext {
        traceparent: Some("00-early-resolve-aaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
        tracestate: Some("tenant=demo".to_string()),
        baggage: None,
    };
    let request = WakeTimerRequest::new(session_id.clone(), request_id.clone(), 1_500)
        .with_trace_context(trace_context.clone());
    append_envelope(
        &stream_url,
        "ds-dp-composite-requested",
        wake_timer_request_envelope(request.clone())?,
    )
    .await?;

    let context = WorkflowContext::new(stream_url.clone());
    let resolver = AwakeableResolver::new(
        stream_url.clone(),
        json_producer(&stream_url, "ds-dp-composite-resolve"),
    );
    let runtime = ControlledWakeTimerRuntime::new(1_000);
    let subscriber = Arc::new(WakeTimerSubscriber::with_runtime(runtime.clone()));

    let waiter = tokio::spawn({
        let context = context.clone();
        let session_id = session_id.clone();
        let request_id = request_id.clone();
        async move {
            context
                .prompt_awakeable::<Value>(session_id, request_id)
                .await
        }
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let timer = {
        let subscriber = Arc::clone(&subscriber);
        let request = request.clone();
        tokio::spawn(async move { subscriber.handle(request).await })
    };

    runtime.wait_for_sleep_started().await;
    resolver
        .resolve_awakeable(
            key.clone(),
            json!({ "winner": "resolve" }),
            Some(trace_context.clone()),
        )
        .await
        .context("resolve prompt wait before timer deadline")?;

    let resolved = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .context("timed out waiting for early resolution winner")?
        .context("awakeable waiter panicked")??;
    assert_eq!(
        resolved.get("winner").and_then(Value::as_str),
        Some("resolve"),
        "INVARIANT (DSV-02): explicit prompt wait must replay/live-converge on the durable resolution winner before a pending timer fires",
    );

    subscriber
        .cancel(session_id.clone(), request_id.clone())
        .await
        .expect("resolving early should allow the pending timer branch to cancel cleanly");
    let timer_outcome = timer.await.context("join early-resolve timer task")?;
    let timer_error = match timer_outcome {
        HandlerOutcome::Failed(error) => error
            .downcast::<WakeTimerHandleError>()
            .expect("canceled timer branch should downcast"),
        HandlerOutcome::Completed(completion) => {
            panic!("timer branch should have been canceled after early resolve: {completion:?}")
        }
        HandlerOutcome::RetryTransient(error) => {
            panic!("timer branch should not retry after early resolve: {error:#}")
        }
    };
    assert_eq!(
        timer_error,
        WakeTimerHandleError::Canceled { key: key.clone() },
        "INVARIANT (DSV-01): once the prompt wait resolves, the competing timer branch must not emit a second terminal event for the same key",
    );

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(
        count_kind_for_key(&rows, &key, WAKE_TIMER_REQUESTED_KIND)?,
        1
    );
    assert_eq!(count_kind_for_key(&rows, &key, AWAKEABLE_WAITING_KIND)?, 1);
    assert_eq!(count_kind_for_key(&rows, &key, AWAKEABLE_RESOLVED_KIND)?, 1);
    assert_eq!(
        count_kind_for_key(&rows, &key, TIMER_FIRED_KIND)?,
        0,
        "INVARIANT (DSV-01): canceling the losing timer branch must avoid a zombie timer_fired row",
    );
    assert_eq!(
        traceparent(
            &row_for_kind(&rows, &key, AWAKEABLE_RESOLVED_KIND)?
                .expect("resolved row should exist"),
        ),
        trace_context.traceparent.as_deref(),
        "INVARIANT (DSV-05): the winning awakeable resolution must preserve trace context",
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn session_teardown_style_cancellation_stops_timer_with_existing_prompt_wait() -> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("ds-dp-composite-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;

    let session_id = SessionId::from("session-cancel");
    let request_id = RequestId::from("request-cancel".to_string());
    let key = CompletionKey::prompt(session_id.clone(), request_id.clone());
    append_waiting(&stream_url, &key, None).await?;

    let request = WakeTimerRequest::new(session_id.clone(), request_id.clone(), 2_000);
    append_envelope(
        &stream_url,
        "ds-dp-composite-requested",
        wake_timer_request_envelope(request.clone())?,
    )
    .await?;

    let runtime = ControlledWakeTimerRuntime::new(1_000);
    let subscriber = Arc::new(WakeTimerSubscriber::with_runtime(runtime.clone()));
    let timer = {
        let subscriber = Arc::clone(&subscriber);
        tokio::spawn(async move { subscriber.handle(request).await })
    };

    runtime.wait_for_sleep_started().await;
    // Phase 6 exposes explicit cancellation; session teardown should call the
    // same seam rather than letting a detached timer emit after the wait is gone.
    subscriber
        .cancel(session_id.clone(), request_id.clone())
        .await
        .expect("session-teardown-style cancellation should succeed for a pending timer");

    let timer_outcome = timer.await.context("join canceled timer task")?;
    let timer_error = match timer_outcome {
        HandlerOutcome::Failed(error) => error
            .downcast::<WakeTimerHandleError>()
            .expect("canceled timer branch should downcast"),
        HandlerOutcome::Completed(completion) => {
            panic!("canceled timer branch must not complete: {completion:?}")
        }
        HandlerOutcome::RetryTransient(error) => {
            panic!("canceled timer branch must not retry: {error:#}")
        }
    };
    assert_eq!(
        timer_error,
        WakeTimerHandleError::Canceled { key: key.clone() },
        "INVARIANT (DSV-10): teardown of a waiting prompt must suppress later timer delivery for the same key",
    );

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(count_kind_for_key(&rows, &key, AWAKEABLE_WAITING_KIND)?, 1);
    assert_eq!(
        count_kind_for_key(&rows, &key, WAKE_TIMER_REQUESTED_KIND)?,
        1
    );
    assert_eq!(
        count_kind_for_key(&rows, &key, TIMER_FIRED_KIND)?,
        0,
        "INVARIANT (DSV-10): a canceled timer must not leave a zombie timer_fired row behind",
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn concurrent_same_key_timer_schedule_converges_to_one_durable_timeout_winner() -> Result<()>
{
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("ds-dp-composite-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;

    let session_id = SessionId::from("session-concurrent");
    let request_id = RequestId::from("request-concurrent".to_string());
    let key = CompletionKey::prompt(session_id.clone(), request_id.clone());
    let trace_context = TraceContext {
        traceparent: Some("00-concurrent-aaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
        tracestate: None,
        baggage: Some("lane=ds-dp-composite".to_string()),
    };
    append_waiting(&stream_url, &key, Some(trace_context.clone())).await?;
    let request = WakeTimerRequest::new(session_id.clone(), request_id.clone(), 1_400)
        .with_trace_context(trace_context.clone());
    append_envelope(
        &stream_url,
        "ds-dp-composite-requested",
        wake_timer_request_envelope(request.clone())?,
    )
    .await?;

    let runtime = ControlledWakeTimerRuntime::new(1_000);
    let subscriber = Arc::new(WakeTimerSubscriber::with_runtime(runtime.clone()));
    let first = {
        let subscriber = Arc::clone(&subscriber);
        let request = request.clone();
        tokio::spawn(async move { subscriber.handle(request).await })
    };
    let second = {
        let subscriber = Arc::clone(&subscriber);
        let request = request.clone();
        tokio::spawn(async move { subscriber.handle(request).await })
    };

    runtime.wait_for_sleep_started().await;
    runtime.release_sleep();

    let outcomes = [
        first.await.context("join first concurrent timer task")?,
        second.await.context("join second concurrent timer task")?,
    ];

    let mut timer_completion = None;
    let mut already_fired = 0;
    for outcome in outcomes {
        match outcome {
            HandlerOutcome::Completed(completion) => timer_completion = Some(completion),
            HandlerOutcome::Failed(error) => {
                let error = error
                    .downcast::<WakeTimerHandleError>()
                    .expect("same-key timer collision should downcast");
                assert_eq!(
                    error,
                    WakeTimerHandleError::AlreadyFired {
                        key: key.clone(),
                        fired_at_ms: 1_400,
                    },
                    "INVARIANT (DSV-01): same-key concurrent timer schedules must converge to one terminal winner and one AlreadyFired loser",
                );
                already_fired += 1;
            }
            HandlerOutcome::RetryTransient(error) => {
                panic!("same-key concurrent timer schedules should converge, not retry: {error:#}")
            }
        }
    }
    let timer_completion =
        timer_completion.expect("one of the concurrent same-key timer tasks must win");
    assert_eq!(already_fired, 1);
    append_envelope(
        &stream_url,
        "ds-dp-composite-fired",
        timer_fired_envelope(timer_completion.clone())?,
    )
    .await?;

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(count_kind_for_key(&rows, &key, AWAKEABLE_WAITING_KIND)?, 1);
    assert_eq!(
        count_kind_for_key(&rows, &key, WAKE_TIMER_REQUESTED_KIND)?,
        1
    );
    assert_eq!(
        count_kind_for_key(&rows, &key, TIMER_FIRED_KIND)?,
        1,
        "INVARIANT (DSV-01): one same-key timer winner must persist exactly one timer_fired row",
    );
    assert_eq!(
        traceparent(
            &row_for_kind(&rows, &key, TIMER_FIRED_KIND)?.expect("timer_fired row should exist"),
        ),
        trace_context.traceparent.as_deref(),
        "INVARIANT (DSV-05): the durable timeout winner must preserve trace context",
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn explicit_timeout_race_leaves_late_resolution_durable_under_current_phase_5_6_composition(
) -> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("ds-dp-composite-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;

    let session_id = SessionId::from("session-timeout");
    let request_id = RequestId::from("request-timeout".to_string());
    let key = CompletionKey::prompt(session_id.clone(), request_id.clone());
    let trace_context = TraceContext {
        traceparent: Some("00-timeout-aaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
        tracestate: Some("source=timeout-race".to_string()),
        baggage: None,
    };
    append_waiting(&stream_url, &key, Some(trace_context.clone())).await?;
    let request = WakeTimerRequest::new(session_id.clone(), request_id.clone(), 1_050)
        .with_trace_context(trace_context.clone());
    append_envelope(
        &stream_url,
        "ds-dp-composite-requested",
        wake_timer_request_envelope(request.clone())?,
    )
    .await?;

    let context = WorkflowContext::new(stream_url.clone());
    let subscriber = Arc::new(WakeTimerSubscriber::with_runtime(
        RecordingWakeTimerRuntime::new(1_000),
    ));

    let race_winner =
        race_prompt_wait_and_timer(context.clone(), Arc::clone(&subscriber), request.clone())
            .await?;
    let timer_completion = match race_winner {
        CompositeRaceOutcome::TimedOut(completion) => completion,
        CompositeRaceOutcome::Resolved(value) => {
            panic!("timer branch should win the explicit timeout race, got resolution: {value:?}")
        }
    };
    append_envelope(
        &stream_url,
        "ds-dp-composite-fired",
        timer_fired_envelope(timer_completion.clone())?,
    )
    .await?;

    assert!(
        tokio::time::timeout(
            Duration::from_millis(75),
            WorkflowContext::new(stream_url.clone()).prompt_awakeable::<Value>(
                session_id.clone(),
                request_id.clone()
            )
        )
        .await
        .is_err(),
        "Phase 5/6 current contract: timer_fired participates in explicit Promise.race-style composition, but it does not yet consume the awakeable key by itself",
    );

    AwakeableResolver::new(
        stream_url.clone(),
        json_producer(&stream_url, "ds-dp-composite-resolve"),
    )
    .resolve_awakeable(
        key.clone(),
        json!({ "winner": "late-resolution" }),
        Some(trace_context.clone()),
    )
    .await
    .context("late resolve after timeout-race winner")?;

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(count_kind_for_key(&rows, &key, AWAKEABLE_WAITING_KIND)?, 1);
    assert_eq!(
        count_kind_for_key(&rows, &key, WAKE_TIMER_REQUESTED_KIND)?,
        1
    );
    assert_eq!(count_kind_for_key(&rows, &key, TIMER_FIRED_KIND)?, 1);
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_RESOLVED_KIND)?,
        1,
        "Phase 5/6 current contract: late resolve remains a distinct durable completion path until timeout sugar fuses the branches at the API level",
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn replay_preserves_pending_timer_deadline_with_existing_prompt_wait() -> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("ds-dp-composite-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;

    let session_id = SessionId::from("session-replay");
    let request_id = RequestId::from("request-replay".to_string());
    let key = CompletionKey::prompt(session_id.clone(), request_id.clone());
    let trace_context = TraceContext {
        traceparent: Some("00-replay-aaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
        tracestate: None,
        baggage: Some("phase=replay".to_string()),
    };
    append_waiting(&stream_url, &key, Some(trace_context.clone())).await?;
    let request = WakeTimerRequest::new(session_id.clone(), request_id.clone(), 2_000)
        .with_trace_context(trace_context.clone());
    append_envelope(
        &stream_url,
        "ds-dp-composite-requested",
        wake_timer_request_envelope(request.clone())?,
    )
    .await?;

    // Phase 6 replay is modeled by instantiating a fresh timer subscriber later
    // with the original durable request envelope and deriving the remaining delay
    // from fire_at_ms rather than any process-local sleep state.
    let runtime = RecordingWakeTimerRuntime::new(1_750);
    let subscriber = WakeTimerSubscriber::with_runtime(runtime.clone());
    let completion = extract_timer_completion(subscriber.handle(request).await)?;
    append_envelope(
        &stream_url,
        "ds-dp-composite-fired",
        timer_fired_envelope(completion.clone())?,
    )
    .await?;

    assert_eq!(
        runtime.sleeps(),
        vec![Duration::from_millis(250)],
        "INVARIANT (DSV-02): replay must preserve the original fire_at deadline instead of resetting the timer after restart",
    );
    assert_eq!(completion.fired_at_ms, 2_000);

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(count_kind_for_key(&rows, &key, AWAKEABLE_WAITING_KIND)?, 1);
    assert_eq!(
        count_kind_for_key(&rows, &key, WAKE_TIMER_REQUESTED_KIND)?,
        1
    );
    assert_eq!(count_kind_for_key(&rows, &key, TIMER_FIRED_KIND)?, 1);
    assert_eq!(
        traceparent(
            &row_for_kind(&rows, &key, TIMER_FIRED_KIND)?
                .expect("replayed timer_fired row should exist"),
        ),
        trace_context.traceparent.as_deref(),
        "INVARIANT (DSV-05): replayed timer completions must preserve the original trace context",
    );

    server.shutdown().await;
    Ok(())
}

async fn race_prompt_wait_and_timer<R>(
    context: WorkflowContext,
    subscriber: Arc<WakeTimerSubscriber<R>>,
    request: WakeTimerRequest,
) -> Result<CompositeRaceOutcome<Value>>
where
    R: WakeTimerRuntime + 'static,
{
    let session_id = request.session_id.clone();
    let request_id = request.request_id.clone();
    tokio::select! {
        resolved = context.prompt_awakeable::<Value>(session_id, request_id) => {
            Ok(CompositeRaceOutcome::Resolved(resolved?))
        }
        outcome = async move { extract_timer_completion(subscriber.handle(request).await) } => {
            Ok(CompositeRaceOutcome::TimedOut(outcome?))
        }
    }
}

fn extract_timer_completion(
    outcome: HandlerOutcome<fireline_harness::TimerFired>,
) -> Result<fireline_harness::TimerFired> {
    match outcome {
        HandlerOutcome::Completed(completion) => Ok(completion),
        HandlerOutcome::Failed(error) => Err(anyhow!("wake timer failed: {error:#}")),
        HandlerOutcome::RetryTransient(error) => Err(anyhow!(
            "wake timer should not retry in composite test: {error:#}"
        )),
    }
}

async fn ensure_json_stream_exists(stream_url: &str) -> Result<()> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(stream_url);
    stream
        .create_with(CreateOptions::new().content_type("application/json"))
        .await
        .map(|_| ())
        .or_else(|error| match error {
            durable_streams::StreamError::Conflict => Ok(()),
            other => Err(other),
        })
        .with_context(|| format!("create durable stream '{stream_url}'"))
}

fn json_producer(stream_url: &str, producer_name: &str) -> Producer {
    let client = DurableStreamsClient::new();
    let mut stream = client.stream(stream_url);
    stream.set_content_type("application/json");
    stream
        .producer(format!("{producer_name}-{}", Uuid::new_v4()))
        .content_type("application/json")
        .build()
}

async fn append_envelope(
    stream_url: &str,
    producer_name: &str,
    envelope: StreamEnvelope,
) -> Result<()> {
    let producer = json_producer(stream_url, producer_name);
    producer.append_json(&envelope);
    producer
        .flush()
        .await
        .with_context(|| format!("flush stream envelope '{}' to '{stream_url}'", envelope.key))
}

async fn append_waiting(
    stream_url: &str,
    key: &CompletionKey,
    trace_context: Option<TraceContext>,
) -> Result<()> {
    let mut envelope = awakeable_waiting_envelope(key.clone())?;
    if let Some(trace_context) = trace_context {
        envelope
            .value
            .as_mut()
            .and_then(Value::as_object_mut)
            .expect("waiting envelope should encode as JSON object")
            .insert(
                "_meta".to_string(),
                Value::Object(trace_context.into_meta()),
            );
    }
    append_envelope(stream_url, "ds-dp-composite-waiting", envelope).await
}

async fn read_all_rows(stream_url: &str) -> Result<Vec<Value>> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(stream_url);
    let mut reader = stream
        .read()
        .offset(Offset::Beginning)
        .live(LiveMode::Off)
        .build()
        .with_context(|| format!("build durable stream reader for '{stream_url}'"))?;

    let mut rows = Vec::new();
    loop {
        let Some(chunk) = reader
            .next_chunk()
            .await
            .with_context(|| format!("read durable stream '{stream_url}'"))?
        else {
            break;
        };
        if !chunk.data.is_empty() {
            rows.extend(
                serde_json::from_slice::<Vec<Value>>(&chunk.data)
                    .context("decode durable stream rows as JSON")?,
            );
        }
        if chunk.up_to_date {
            break;
        }
    }
    Ok(rows)
}

fn count_kind_for_key(rows: &[Value], key: &CompletionKey, kind: &str) -> Result<usize> {
    Ok(rows
        .iter()
        .cloned()
        .map(|row| StreamEnvelope::from_json(row).context("decode composite stream envelope"))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|envelope| {
            envelope.kind() == Some(kind) && envelope.completion_key().as_ref() == Some(key)
        })
        .count())
}

fn row_for_kind(rows: &[Value], key: &CompletionKey, kind: &str) -> Result<Option<StreamEnvelope>> {
    Ok(rows
        .iter()
        .cloned()
        .map(|row| StreamEnvelope::from_json(row).context("decode composite stream envelope"))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .find(|envelope| {
            envelope.kind() == Some(kind) && envelope.completion_key().as_ref() == Some(key)
        }))
}

fn traceparent(envelope: &StreamEnvelope) -> Option<&str> {
    envelope
        .value
        .as_ref()
        .and_then(|value| value.get("_meta"))
        .and_then(|meta| meta.get("traceparent"))
        .and_then(Value::as_str)
}
