use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, Producer, StreamError};
use fireline_harness::{
    AwakeableKey, AwakeableSubscriber, DurableSubscriberDriver, SubscriberMode,
    SubscriberRegistration, WorkflowContext, awakeable_resolution_envelope,
};
use sacp::schema::{RequestId, SessionId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalDecision {
    allow: bool,
    reviewer: String,
}

#[tokio::test]
async fn workflow_context_awakeable_resolves_typed_value_via_passive_subscriber() -> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("awakeable-basic-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let producer = json_producer(&stream_url, "awakeable-basic");

    let mut subscriber_driver = DurableSubscriberDriver::new();
    subscriber_driver.register_passive(AwakeableSubscriber::new());
    assert_eq!(
        subscriber_driver.registrations(),
        vec![SubscriberRegistration {
            name: AwakeableSubscriber::NAME.to_string(),
            mode: SubscriberMode::Passive,
        }],
        "INVARIANT (DurablePromises): Rust workflow awakeables must be backed by the passive durable-subscriber substrate",
    );

    let context =
        WorkflowContext::with_subscriber_driver(stream_url.clone(), Arc::new(subscriber_driver));
    let key = AwakeableKey::prompt(
        SessionId::from("session-awakeable"),
        RequestId::from("request-awakeable".to_string()),
    );

    let awakeable = context.awakeable::<ApprovalDecision>(key.clone());
    assert_eq!(awakeable.key(), &key);
    let waiter = tokio::spawn(async move { awakeable.await });

    tokio::time::sleep(Duration::from_millis(100)).await;
    producer.append_json(&awakeable_resolution_envelope(
        key,
        ApprovalDecision {
            allow: true,
            reviewer: "ops-oncall".to_string(),
        },
    )?);
    producer
        .flush()
        .await
        .context("flush awakeable_resolved completion")?;

    let resolved = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .context("timed out waiting for awakeable future")?
        .context("awakeable waiter panicked")??;
    assert_eq!(
        resolved,
        ApprovalDecision {
            allow: true,
            reviewer: "ops-oncall".to_string(),
        },
        "INVARIANT (DurablePromises): matching awakeable_resolved payload must deserialize back into the caller's typed Rust surface",
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
                    "retrying awakeable test stream creation"
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
