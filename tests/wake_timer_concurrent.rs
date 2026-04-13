use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fireline_harness::{
    ActiveSubscriber, CompletionKey, HandlerOutcome, TraceContext, WakeTimerHandleError,
    WakeTimerRequest, WakeTimerSubscriber,
};
use sacp::schema::{RequestId, SessionId};

#[path = "support/wake_timer_runtime.rs"]
mod wake_timer_runtime;

use wake_timer_runtime::ControlledWakeTimerRuntime;

#[tokio::test]
async fn wake_timer_same_completion_key_converges_to_single_timer_fired() -> Result<()> {
    let runtime = ControlledWakeTimerRuntime::new(1_000);
    let subscriber = Arc::new(WakeTimerSubscriber::with_runtime(runtime.clone()));
    let session_id = SessionId::from("session-concurrent");
    let request_id = RequestId::from("request-concurrent".to_string());
    let key = CompletionKey::prompt(session_id.clone(), request_id.clone());
    let event = WakeTimerRequest::new(session_id.clone(), request_id.clone(), 1_400)
        .with_trace_context(TraceContext {
            traceparent: Some(
                "00-concurrent-trace-aaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".into(),
            ),
            tracestate: None,
            baggage: None,
        });

    let first = {
        let subscriber = Arc::clone(&subscriber);
        let event = event.clone();
        tokio::spawn(async move { subscriber.handle(event).await })
    };
    let second = {
        let subscriber = Arc::clone(&subscriber);
        let event = event.clone();
        tokio::spawn(async move { subscriber.handle(event).await })
    };

    runtime.wait_for_sleep_started().await;
    runtime.release_sleep();

    let outcomes = [
        first.await.expect("first timer task should not panic"),
        second.await.expect("second timer task should not panic"),
    ];

    assert_eq!(
        runtime.sleeps(),
        vec![Duration::from_millis(400)],
        "INVARIANT (DSV-01): only one active timer should own a given completion key",
    );

    let mut completions = 0;
    let mut already_fired = 0;
    for outcome in outcomes {
        match outcome {
            HandlerOutcome::Completed(completion) => {
                completions += 1;
                assert_eq!(completion.completion_key(), key);
                assert_eq!(completion.fired_at_ms, 1_400);
                assert_eq!(
                    completion.trace_context.traceparent.as_deref(),
                    Some("00-concurrent-trace-aaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01"),
                    "INVARIANT (DSV-05): the winning timer_fired completion must preserve trace context",
                );
            }
            HandlerOutcome::Failed(error) => {
                let error = error
                    .downcast::<WakeTimerHandleError>()
                    .expect("wake timer collision failure should downcast");
                assert_eq!(
                    error,
                    WakeTimerHandleError::AlreadyFired {
                        key: key.clone(),
                        fired_at_ms: 1_400,
                    },
                );
                already_fired += 1;
            }
            HandlerOutcome::RetryTransient(error) => {
                panic!("same-key wake timer collision should converge, not retry: {error:#}")
            }
        }
    }

    assert_eq!(completions, 1);
    assert_eq!(already_fired, 1);

    Ok(())
}
