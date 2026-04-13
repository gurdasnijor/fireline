use std::time::Duration;

use anyhow::Result;
use fireline_harness::{
    ActiveSubscriber, DurableSubscriber, HandlerOutcome, TimerFired, WakeTimerRequest,
    WakeTimerSubscriber, timer_fired_envelope,
};
use sacp::schema::{RequestId, SessionId};

#[path = "support/wake_timer_runtime.rs"]
mod wake_timer_runtime;

use wake_timer_runtime::RecordingWakeTimerRuntime;

#[tokio::test]
async fn wake_timer_subscriber_replay_restores_pending_wait_using_remaining_delay() -> Result<()> {
    let runtime = RecordingWakeTimerRuntime::new(2_200);
    let subscriber = WakeTimerSubscriber::with_runtime(runtime.clone());
    let event = WakeTimerRequest::new(
        SessionId::from("session-replay-pending"),
        RequestId::from("request-replay-pending".to_string()),
        2_700,
    );

    let outcome = subscriber.handle(event).await;
    let completion = match outcome {
        HandlerOutcome::Completed(completion) => completion,
        HandlerOutcome::RetryTransient(error) | HandlerOutcome::Failed(error) => {
            panic!("replayed pending timer should complete successfully: {error:#}")
        }
    };

    assert_eq!(runtime.sleeps(), vec![Duration::from_millis(500)]);
    assert_eq!(
        completion.fired_at_ms, 2_700,
        "INVARIANT (DS Phase 6): replayed pending timers must resume from the remaining delay rather than resetting the deadline",
    );

    Ok(())
}

#[test]
fn wake_timer_subscriber_detects_existing_completion_from_replay_log() -> Result<()> {
    let subscriber = WakeTimerSubscriber::new();
    let event = WakeTimerRequest::new(
        SessionId::from("session-replay-fired"),
        RequestId::from("request-replay-fired".to_string()),
        1_600,
    );
    let log = vec![timer_fired_envelope(TimerFired::new(
        SessionId::from("session-replay-fired"),
        RequestId::from("request-replay-fired".to_string()),
        1_600,
    ))?];

    assert!(
        subscriber.is_completed(&event, &log),
        "INVARIANT (DS Phase 6): replay after an existing timer_fired must not schedule or fire the timer again",
    );

    Ok(())
}
