use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, Producer, StreamError};
use fireline_harness::{
    AwakeableKey, AwakeableSubscriber, DurableSubscriberDriver, WorkflowContext,
    awakeable_resolution_envelope,
};
use sacp::schema::{RequestId, SessionId};
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

#[tokio::test]
async fn workflow_context_rehydrates_without_rewaiting_after_completion_is_already_present()
-> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("awakeable-replay-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let producer = json_producer(&stream_url, "awakeable-replay");

    let key = AwakeableKey::prompt(
        SessionId::from("session-replay"),
        RequestId::from("request-replay".to_string()),
    );
    producer.append_json(&awakeable_resolution_envelope(key.clone(), true)?);
    producer
        .flush()
        .await
        .context("flush preexisting awakeable_resolved completion")?;

    let mut subscriber_driver = DurableSubscriberDriver::new();
    subscriber_driver.register_passive(AwakeableSubscriber::new());
    let context =
        WorkflowContext::with_subscriber_driver(stream_url.clone(), Arc::new(subscriber_driver));

    let resolved = tokio::time::timeout(
        Duration::from_millis(250),
        context.awakeable::<bool>(key),
    )
    .await
    .context("rehydrated awakeable should resolve from replay without waiting for another completion")??;

    assert!(
        resolved,
        "INVARIANT (DurablePromises): recreating the workflow context after completion exists must not re-await an already-resolved awakeable",
    );

    server.shutdown().await;
    Ok(())
}

async fn ensure_json_stream_exists(stream_url: &str) -> Result<()> {
    let client = DurableStreamsClient::new();
    let stream = client.stream(stream_url);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match stream
            .create_with(CreateOptions::new().content_type("application/json"))
            .await
        {
            Ok(_) | Err(StreamError::Conflict) => return Ok(()),
            Err(error) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
                tracing::debug!(
                    ?error,
                    stream_url,
                    "retrying awakeable replay stream creation"
                );
            }
            Err(error) => return Err(error.into()),
        }
    }
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
