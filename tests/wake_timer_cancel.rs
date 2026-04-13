use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fireline_harness::{
    ActiveSubscriber, CompletionKey, HandlerOutcome, WakeTimerCancelError, WakeTimerHandleError,
    WakeTimerRequest, WakeTimerSubscriber,
};
use sacp::schema::{RequestId, SessionId};

#[path = "support/wake_timer_runtime.rs"]
mod wake_timer_runtime;

use wake_timer_runtime::{ControlledWakeTimerRuntime, RecordingWakeTimerRuntime};

#[tokio::test]
async fn wake_timer_cancel_before_fire_stops_delivery_without_appending_completion() -> Result<()> {
    let runtime = ControlledWakeTimerRuntime::new(1_000);
    let subscriber = Arc::new(WakeTimerSubscriber::with_runtime(runtime.clone()));
    let session_id = SessionId::from("session-cancel");
    let request_id = RequestId::from("request-cancel".to_string());
    let key = CompletionKey::prompt(session_id.clone(), request_id.clone());
    let handle = {
        let subscriber = Arc::clone(&subscriber);
        let session_id = session_id.clone();
        let request_id = request_id.clone();
        tokio::spawn(async move {
            subscriber
                .handle(WakeTimerRequest::new(session_id, request_id, 2_000))
                .await
        })
    };

    runtime.wait_for_sleep_started().await;
    subscriber
        .cancel(session_id.clone(), request_id.clone())
        .await
        .expect("pending timer should cancel");

    let outcome = handle.await.expect("wake timer task should not panic");
    let error = match outcome {
        HandlerOutcome::Failed(error) => error
            .downcast::<WakeTimerHandleError>()
            .expect("wake timer failure should downcast"),
        HandlerOutcome::Completed(completion) => {
            panic!("canceled timer should not complete: {completion:?}")
        }
        HandlerOutcome::RetryTransient(error) => {
            panic!("canceled timer should not retry: {error:#}")
        }
    };

    assert_eq!(
        error,
        WakeTimerHandleError::Canceled { key: key.clone() },
        "INVARIANT (DS Phase 6): cancel-before-fire must stop the active profile without emitting timer_fired",
    );
    assert_eq!(runtime.sleeps(), vec![Duration::from_millis(1_000)]);
    assert!(
        subscriber.cancel(session_id, request_id).await.is_ok(),
        "re-canceling a canceled timer should stay idempotent",
    );

    Ok(())
}

#[tokio::test]
async fn wake_timer_cancel_after_fire_reports_already_fired() -> Result<()> {
    let runtime = RecordingWakeTimerRuntime::new(1_000);
    let subscriber = WakeTimerSubscriber::with_runtime(runtime);
    let session_id = SessionId::from("session-fired");
    let request_id = RequestId::from("request-fired".to_string());
    let key = CompletionKey::prompt(session_id.clone(), request_id.clone());

    let outcome = subscriber
        .handle(WakeTimerRequest::new(
            session_id.clone(),
            request_id.clone(),
            1_250,
        ))
        .await;
    match outcome {
        HandlerOutcome::Completed(_) => {}
        HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
            panic!("wake timer should have fired before cancel-after-fire check: {error:#}")
        }
    }

    let error = subscriber
        .cancel(session_id, request_id)
        .await
        .expect_err("cancel-after-fire must report AlreadyFired");
    assert_eq!(
        error,
        WakeTimerCancelError::AlreadyFired {
            key,
            fired_at_ms: 1_250,
        },
        "INVARIANT (DS Phase 6): once timer_fired is durable, later cancellation attempts must not resurrect or suppress it",
    );

    Ok(())
}
