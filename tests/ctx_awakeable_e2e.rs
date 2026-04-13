use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, LiveMode, Offset, Producer};
use fireline_harness::{
    AWAKEABLE_RESOLVED_KIND, AwakeableKey, AwakeableResolver, AwakeableSubscriber,
    DurableSubscriberDriver, StreamEnvelope, SubscriberMode, SubscriberRegistration,
    TraceContext, WorkflowContext,
};
use sacp::schema::{RequestId, SessionId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
async fn ctx_awakeable_prompt_scope_resolves_via_resolver_with_trace_context() -> Result<()> {
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("ctx-awakeable-e2e-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;

    let mut subscriber_driver = DurableSubscriberDriver::new();
    subscriber_driver.register_passive(AwakeableSubscriber::new());
    assert_eq!(
        subscriber_driver.registrations(),
        vec![SubscriberRegistration {
            name: AwakeableSubscriber::NAME.to_string(),
            mode: SubscriberMode::Passive,
        }],
        "INVARIANT (DurablePromises): ctx.awakeable<T>() must stay backed by the passive durable-subscriber substrate",
    );

    let context =
        WorkflowContext::with_subscriber_driver(stream_url.clone(), Arc::new(subscriber_driver));
    let resolver = AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "ctx-e2e"));
    let key = AwakeableKey::prompt(
        SessionId::from("session-e2e".to_string()),
        RequestId::from("request-e2e".to_string()),
    );
    let expected = ApprovalDecision {
        allow: true,
        reviewer: "ops-oncall".to_string(),
    };
    let trace = TraceContext {
        traceparent: Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
        tracestate: Some("vendor=value".to_string()),
        baggage: Some("actor=ctx-awakeable-e2e".to_string()),
    };

    let waiter = tokio::spawn({
        let key = key.clone();
        async move { context.awakeable::<ApprovalDecision>(key).await }
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    resolver
        .resolve_awakeable(key.clone(), expected.clone(), Some(trace.clone()))
        .await
        .context("resolve prompt-scoped awakeable")?;

    let resolved = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .context("timed out waiting for ctx.awakeable waiter")?
        .context("ctx.awakeable waiter panicked")??;
    assert_eq!(resolved, expected);

    let rows = read_all_rows(&stream_url).await?;
    let resolution = find_resolution_envelope(&rows, &key)?;
    assert_eq!(resolution.kind(), Some(AWAKEABLE_RESOLVED_KIND));
    assert_trace_context(&resolution, &trace);

    let payload = resolution
        .value_as::<fireline_harness::AwakeableResolved<ApprovalDecision>>()
        .ok_or_else(|| anyhow!("decode awakeable_resolved payload for prompt key"))?;
    assert_eq!(payload.value, expected);

    server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn ctx_awakeable_replays_resolution_after_restart_when_waiter_crashes_mid_await() -> Result<()>
{
    let server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = server.stream_url(&format!("ctx-awakeable-replay-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;

    let key = AwakeableKey::prompt(
        SessionId::from("session-replay".to_string()),
        RequestId::from("request-replay".to_string()),
    );
    let expected = ApprovalDecision {
        allow: false,
        reviewer: "release-manager".to_string(),
    };
    let trace = TraceContext {
        traceparent: Some("00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01".to_string()),
        tracestate: Some("tenant=demo".to_string()),
        baggage: Some("phase=replay".to_string()),
    };

    let crashed_waiter = tokio::spawn({
        let context = workflow_context(&stream_url);
        let key = key.clone();
        async move { context.awakeable::<ApprovalDecision>(key).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    crashed_waiter.abort();
    let _ = crashed_waiter.await;

    let resolver =
        AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "ctx-replay"));
    resolver
        .resolve_awakeable(key.clone(), expected.clone(), Some(trace.clone()))
        .await
        .context("resolve prompt-scoped awakeable after simulated crash")?;

    let replayed = tokio::time::timeout(
        Duration::from_millis(250),
        workflow_context(&stream_url).awakeable::<ApprovalDecision>(key.clone()),
    )
    .await
    .context("rehydrated ctx.awakeable should resolve from replay after restart")??;
    assert_eq!(
        replayed, expected,
        "INVARIANT (ReplayIdempotent): rebuilding the workflow context after a crash must observe the pre-crash awakeable resolution exactly once",
    );

    let rows = read_all_rows(&stream_url).await?;
    let resolution = find_resolution_envelope(&rows, &key)?;
    assert_trace_context(&resolution, &trace);
    assert_eq!(
        count_resolutions(&rows, &key)?,
        1,
        "INVARIANT (CompletionKeyUnique): replay-after-crash must not append a second resolution for the same prompt key",
    );

    server.shutdown().await;
    Ok(())
}

fn workflow_context(stream_url: &str) -> WorkflowContext {
    let mut subscriber_driver = DurableSubscriberDriver::new();
    subscriber_driver.register_passive(AwakeableSubscriber::new());
    WorkflowContext::with_subscriber_driver(stream_url.to_string(), Arc::new(subscriber_driver))
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

fn find_resolution_envelope(rows: &[Value], key: &AwakeableKey) -> Result<StreamEnvelope> {
    rows.iter()
        .cloned()
        .map(|row| StreamEnvelope::from_json(row).context("decode awakeable stream envelope"))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .find(|envelope| {
            envelope.kind() == Some(AWAKEABLE_RESOLVED_KIND)
                && envelope.completion_key().as_ref() == Some(key)
        })
        .ok_or_else(|| anyhow!("missing awakeable_resolved envelope for '{}'", key.storage_key()))
}

fn count_resolutions(rows: &[Value], key: &AwakeableKey) -> Result<usize> {
    Ok(rows
        .iter()
        .cloned()
        .map(|row| StreamEnvelope::from_json(row).context("decode awakeable stream envelope"))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|envelope| {
            envelope.kind() == Some(AWAKEABLE_RESOLVED_KIND)
                && envelope.completion_key().as_ref() == Some(key)
        })
        .count())
}

fn assert_trace_context(envelope: &StreamEnvelope, trace: &TraceContext) {
    let value = envelope.value.as_ref().expect("awakeable resolution payload");
    assert_eq!(
        value
            .get("_meta")
            .and_then(|meta| meta.get("traceparent"))
            .and_then(Value::as_str),
        trace.traceparent.as_deref(),
    );
    assert_eq!(
        value
            .get("_meta")
            .and_then(|meta| meta.get("tracestate"))
            .and_then(Value::as_str),
        trace.tracestate.as_deref(),
    );
    assert_eq!(
        value
            .get("_meta")
            .and_then(|meta| meta.get("baggage"))
            .and_then(Value::as_str),
        trace.baggage.as_deref(),
    );
}
