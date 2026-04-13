use std::time::Duration;

use anyhow::{Context, Result};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, Producer, StreamError};
use fireline_harness::{
    AwakeableKey, StreamEnvelope, TraceContext, WorkflowContext, awakeable_resolution_envelope,
};
use sacp::schema::{RequestId, SessionId};
use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

#[tokio::test]
async fn workflow_context_race_returns_the_first_resolved_awakeable() -> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("awakeable-race-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let producer = json_producer(&stream_url, "awakeable-race");
    let context = WorkflowContext::new(stream_url.clone());

    let key_a = AwakeableKey::prompt(
        SessionId::from("session-race"),
        RequestId::from("request-a".to_string()),
    );
    let key_b = AwakeableKey::prompt(
        SessionId::from("session-race"),
        RequestId::from("request-b".to_string()),
    );

    let race_task = tokio::spawn({
        let context = context.clone();
        let key_a = key_a.clone();
        let key_b = key_b.clone();
        async move {
            context
                .race([
                    context.awakeable::<String>(key_a),
                    context.awakeable::<String>(key_b),
                ])
                .await
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    producer.append_json(&resolution_envelope_with_trace(
        key_b.clone(),
        "winner-b".to_string(),
        TraceContext {
            traceparent: Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
            tracestate: Some("vendor=value".to_string()),
            baggage: None,
        },
    )?);
    producer
        .flush()
        .await
        .context("flush winning awakeable resolution")?;

    let winner = tokio::time::timeout(Duration::from_secs(2), race_task)
        .await
        .context("timed out waiting for awakeable race")?
        .context("awakeable race panicked")??;

    assert_eq!(winner.winner_index, 1);
    assert_eq!(winner.winner_key, key_b);
    assert_eq!(winner.value, "winner-b".to_string());
    assert_eq!(
        winner
            .trace_context
            .as_ref()
            .and_then(|meta| meta.traceparent.as_deref()),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")
    );

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn awakeable_race_keeps_losing_branch_valid_for_later_waiters() -> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("awakeable-race-loser-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let producer = json_producer(&stream_url, "awakeable-race-loser");
    let context = WorkflowContext::new(stream_url.clone());

    let key_a = AwakeableKey::prompt(
        SessionId::from("session-race"),
        RequestId::from("request-a".to_string()),
    );
    let key_b = AwakeableKey::prompt(
        SessionId::from("session-race"),
        RequestId::from("request-b".to_string()),
    );

    let winner = tokio::spawn({
        let context = context.clone();
        let key_a = key_a.clone();
        let key_b = key_b.clone();
        async move {
            context
                .race([
                    context.awakeable::<String>(key_a),
                    context.awakeable::<String>(key_b),
                ])
                .await
        }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    producer.append_json(&awakeable_resolution_envelope(
        key_b.clone(),
        "winner-b".to_string(),
    )?);
    producer
        .flush()
        .await
        .context("flush winning awakeable resolution")?;

    let winner = tokio::time::timeout(Duration::from_secs(2), winner)
        .await
        .context("timed out waiting for winner")?
        .context("race task panicked")??;
    assert_eq!(winner.winner_key, key_b);

    let loser_wait = tokio::spawn({
        let context = context.clone();
        let key_a = key_a.clone();
        async move { context.awakeable::<String>(key_a).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    producer.append_json(&awakeable_resolution_envelope(
        key_a,
        "loser-still-resolves".to_string(),
    )?);
    producer
        .flush()
        .await
        .context("flush losing awakeable resolution")?;

    let loser_value = tokio::time::timeout(Duration::from_secs(2), loser_wait)
        .await
        .context("timed out waiting for losing branch")?
        .context("losing branch wait panicked")??;
    assert_eq!(loser_value, "loser-still-resolves".to_string());

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn awakeable_race_requires_at_least_one_branch() -> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("awakeable-race-empty-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let context = WorkflowContext::new(stream_url);

    let error = context
        .race::<String, _>(std::iter::empty())
        .await
        .expect_err("race without branches should be rejected");
    assert!(
        error.to_string().contains("at least one branch"),
        "empty-race error should explain the invariant: {error:#}"
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
                tracing::debug!(?error, stream_url, "retrying awakeable race stream creation");
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

fn resolution_envelope_with_trace<T>(
    key: AwakeableKey,
    value: T,
    trace_context: TraceContext,
) -> Result<StreamEnvelope>
where
    T: Serialize,
{
    let mut envelope = awakeable_resolution_envelope(key, value)?;
    if let Some(payload) = envelope.value.as_mut().and_then(Value::as_object_mut) {
        payload.insert("_meta".to_string(), Value::Object(trace_context.into_meta()));
    }
    Ok(envelope)
}
