use std::time::Duration;

use anyhow::Result;
use fireline_harness::{
    ActiveSubscriber, HandlerOutcome, TimerFired, TraceContext, WakeTimerRequest,
    WakeTimerSubscriber, timer_fired_envelope,
};
use sacp::schema::{RequestId, SessionId};

#[path = "support/wake_timer_runtime.rs"]
mod wake_timer_runtime;

use wake_timer_runtime::RecordingWakeTimerRuntime;

#[tokio::test]
async fn wake_timer_subscriber_fires_prompt_scoped_timer_and_preserves_trace_context() -> Result<()>
{
    let runtime = RecordingWakeTimerRuntime::new(1_250);
    let subscriber = WakeTimerSubscriber::with_runtime(runtime.clone());
    let event = WakeTimerRequest::new(
        SessionId::from("session-basic"),
        RequestId::from("request-basic".to_string()),
        1_600,
    )
    .with_trace_context(TraceContext {
        traceparent: Some("00-basic-trace-aaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".into()),
        tracestate: Some("vendor=value".into()),
        baggage: None,
    });

    let outcome = subscriber.handle(event.clone()).await;
    let completion = match outcome {
        HandlerOutcome::Completed(completion) => completion,
        HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
            panic!("wake timer should complete successfully: {error:#}")
        }
    };

    assert_eq!(runtime.sleeps(), vec![Duration::from_millis(350)]);
    assert_eq!(
        completion,
        TimerFired::new(
            SessionId::from("session-basic"),
            RequestId::from("request-basic".to_string()),
            1_600,
        )
        .with_trace_context(TraceContext {
            traceparent: Some("00-basic-trace-aaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".into()),
            tracestate: Some("vendor=value".into()),
            baggage: None,
        }),
        "INVARIANT (DS Phase 6): timer_fired must stay prompt-scoped and carry the originating trace context",
    );

    let envelope = timer_fired_envelope(completion)?;
    assert_eq!(envelope.kind(), Some("timer_fired"));
    assert_eq!(envelope.completion_key(), Some(event.completion_key()));
    assert_eq!(
        envelope.trace_context(),
        Some(TraceContext {
            traceparent: Some("00-basic-trace-aaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".into()),
            tracestate: Some("vendor=value".into()),
            baggage: None,
        }),
        "INVARIANT (DSV-05): timer_fired envelopes must preserve W3C trace context",
    );

    Ok(())
}
