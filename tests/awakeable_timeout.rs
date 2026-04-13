use std::time::Duration;

use anyhow::Result;
use fireline_harness::{AwakeableKey, AwakeableTimeoutError, WorkflowContext};
use sacp::schema::{RequestId, SessionId};
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

#[tokio::test]
async fn timeout_helper_stays_blocked_until_timeout_key_invariant_is_locked() -> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("awakeable-timeout-blocked-{}", Uuid::new_v4()));
    let context = WorkflowContext::new(stream_url);
    let key = AwakeableKey::prompt(
        SessionId::from("session-timeout"),
        RequestId::from("request-timeout".to_string()),
    );

    let error = context
        .awakeable::<bool>(key)
        .with_timeout(Duration::from_secs(30))
        .await
        .expect_err("timeout helper should stay blocked until DS Phase 6 lands WakeTimerSubscriber");

    assert_eq!(error, AwakeableTimeoutError::RequiresWakeTimerSubscriber);

    server.shutdown().await;
    Ok(())
}
