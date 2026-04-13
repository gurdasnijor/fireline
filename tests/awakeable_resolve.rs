use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, LiveMode, Offset, Producer};
use fireline_harness::{
    AWAKEABLE_REJECTED_KIND, AWAKEABLE_RESOLVED_KIND, AWAKEABLE_WAITING_KIND,
    AwakeableResolved, AwakeableResolver, AwakeableSubscriber, CompletionKey,
    DurableSubscriberDriver, ResolveError, StreamEnvelope, TraceContext, WorkflowContext,
};
use sacp::schema::{RequestId, SessionId, ToolCallId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

#[tokio::test]
async fn resolve_awakeable_appends_canonical_prompt_completion_with_traceparent() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("awakeable-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let resolver =
        AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "awakeable"));
    let key = CompletionKey::prompt(
        SessionId::from("session-a".to_string()),
        RequestId::from("req-1".to_string()),
    );

    resolver
        .resolve_awakeable(
            key.clone(),
            json!({ "allowed": true }),
            Some(TraceContext {
                traceparent: Some(
                    "00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string(),
                ),
                tracestate: Some("vendor=value".to_string()),
                baggage: None,
            }),
        )
        .await
        .expect("resolve prompt awakeable");

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(rows.len(), 1);

    let envelope =
        StreamEnvelope::from_json(rows[0].clone()).context("decode awakeable envelope")?;
    assert_eq!(envelope.entity_type, "awakeable");
    assert_eq!(envelope.kind(), Some(AWAKEABLE_RESOLVED_KIND));
    assert_eq!(envelope.completion_key(), Some(key));

    let value = envelope
        .value
        .as_ref()
        .context("awakeable resolution value")?;
    assert_eq!(
        value
            .get("_meta")
            .and_then(|meta| meta.get("traceparent"))
            .and_then(Value::as_str),
        Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01")
    );
    assert_eq!(
        value
            .get("value")
            .and_then(|payload| payload.get("allowed"))
            .and_then(Value::as_bool),
        Some(true)
    );

    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn reject_awakeable_appends_canonical_rejection_with_traceparent_and_wakes_waiter()
-> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("awakeable-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let resolver =
        AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "awakeable"));
    let key = CompletionKey::prompt(
        SessionId::from("session-reject".to_string()),
        RequestId::from("req-reject".to_string()),
    );
    let context = WorkflowContext::new(stream_url.clone());
    let waiter = tokio::spawn({
        let key = key.clone();
        async move { context.awakeable::<bool>(key).await }
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    resolver
        .reject_awakeable(
            key.clone(),
            json!({ "reason": "policy denied" }),
            Some(TraceContext {
                traceparent: Some(
                    "00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01".to_string(),
                ),
                tracestate: None,
                baggage: Some("tenant=ops".to_string()),
            }),
        )
        .await
        .expect("reject prompt awakeable");

    let rejected = tokio::time::timeout(std::time::Duration::from_secs(2), waiter)
        .await
        .context("timed out waiting for rejected awakeable")?
        .context("awakeable waiter panicked")?;
    let error = rejected.expect_err("rejected awakeable must return Err to waiter");
    assert!(
        error.to_string().contains("policy denied"),
        "rejection error should include serialized rejection payload"
    );

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(
        rows.len(),
        2,
        "Phase 4 replay integration should durably persist the waiting row before rejection",
    );

    let waiting = rows
        .iter()
        .cloned()
        .filter_map(|row| StreamEnvelope::from_json(row).ok())
        .find(|envelope| envelope.kind() == Some(AWAKEABLE_WAITING_KIND))
        .context("awakeable waiting envelope")?;
    assert_eq!(waiting.completion_key(), Some(key.clone()));

    let envelope = rows
        .iter()
        .cloned()
        .filter_map(|row| StreamEnvelope::from_json(row).ok())
        .find(|envelope| envelope.kind() == Some(AWAKEABLE_REJECTED_KIND))
        .context("awakeable rejection envelope")?;
    assert_eq!(envelope.entity_type, "awakeable");
    assert_eq!(envelope.kind(), Some(AWAKEABLE_REJECTED_KIND));
    assert_eq!(envelope.completion_key(), Some(key));
    assert_eq!(
        envelope
            .value
            .as_ref()
            .and_then(|value| value.get("_meta"))
            .and_then(|meta| meta.get("traceparent"))
            .and_then(Value::as_str),
        Some("00-cccccccccccccccccccccccccccccccc-dddddddddddddddd-01")
    );

    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn resolve_awakeable_returns_already_resolved_without_second_append() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("awakeable-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let resolver =
        AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "awakeable"));
    let key = CompletionKey::tool(
        SessionId::from("session-b".to_string()),
        ToolCallId::from("tool-1".to_string()),
    );

    resolver
        .resolve_awakeable(key.clone(), json!({ "status": "ok" }), None)
        .await
        .expect("first resolve");
    let second = resolver
        .resolve_awakeable(key.clone(), json!({ "status": "again" }), None)
        .await;

    match second {
        Err(ResolveError::AlreadyResolved(found)) => assert_eq!(found, key),
        other => panic!("expected AlreadyResolved, got {other:?}"),
    }

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(rows.len(), 1, "idempotent resolve should not append twice");

    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn resolve_after_reject_returns_already_resolved_without_second_append() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("awakeable-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let resolver =
        AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "awakeable"));
    let key = CompletionKey::tool(
        SessionId::from("session-rar".to_string()),
        ToolCallId::from("tool-rar".to_string()),
    );

    resolver
        .reject_awakeable(key.clone(), json!({ "status": "denied" }), None)
        .await
        .expect("first reject");
    let second = resolver
        .resolve_awakeable(key.clone(), json!({ "status": "ok" }), None)
        .await;

    match second {
        Err(ResolveError::AlreadyResolved(found)) => assert_eq!(found, key),
        other => panic!("expected AlreadyResolved after reject, got {other:?}"),
    }

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(rows.len(), 1, "reject then resolve should keep one completion row");
    let envelope = StreamEnvelope::from_json(rows[0].clone())?;
    assert_eq!(envelope.kind(), Some(AWAKEABLE_REJECTED_KIND));

    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn reject_after_resolve_returns_already_resolved_without_second_append() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("awakeable-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let resolver =
        AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "awakeable"));
    let key = CompletionKey::session(SessionId::from("session-rar2".to_string()));

    resolver
        .resolve_awakeable(key.clone(), json!({ "winner": "resolve" }), None)
        .await
        .expect("first resolve");
    let second = resolver
        .reject_awakeable(key.clone(), json!({ "winner": "reject" }), None)
        .await;

    match second {
        Err(ResolveError::AlreadyResolved(found)) => assert_eq!(found, key),
        other => panic!("expected AlreadyResolved after resolve, got {other:?}"),
    }

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(rows.len(), 1, "resolve then reject should keep one completion row");
    let envelope = StreamEnvelope::from_json(rows[0].clone())?;
    assert_eq!(envelope.kind(), Some(AWAKEABLE_RESOLVED_KIND));

    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn resolve_awakeable_is_concurrent_safe_for_same_key() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("awakeable-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let resolver =
        AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "awakeable"));
    let key = CompletionKey::session(SessionId::from("session-c".to_string()));

    let first = {
        let resolver = resolver.clone();
        let key = key.clone();
        tokio::spawn(async move {
            resolver
                .resolve_awakeable(key, json!({ "winner": 1 }), None)
                .await
        })
    };
    let second = {
        let resolver = resolver.clone();
        let key = key.clone();
        tokio::spawn(async move {
            resolver
                .resolve_awakeable(key, json!({ "winner": 2 }), None)
                .await
        })
    };

    let first = first.await.expect("first task");
    let second = second.await.expect("second task");

    assert!(
        matches!(
            (&first, &second),
            (Ok(()), Err(ResolveError::AlreadyResolved(_)))
                | (Err(ResolveError::AlreadyResolved(_)), Ok(()))
        ),
        "one concurrent resolver should win and the other should observe AlreadyResolved"
    );

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(rows.len(), 1, "concurrent resolve should persist only one row");

    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn resolve_and_reject_concurrently_converge_to_first_wins() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("awakeable-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let resolver =
        AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "awakeable"));
    let key = CompletionKey::prompt(
        SessionId::from("session-race".to_string()),
        RequestId::from("req-race".to_string()),
    );

    let resolved = {
        let resolver = resolver.clone();
        let key = key.clone();
        tokio::spawn(async move {
            resolver
                .resolve_awakeable(key, json!({ "winner": "resolve" }), None)
                .await
        })
    };
    let rejected = {
        let resolver = resolver.clone();
        let key = key.clone();
        tokio::spawn(async move {
            resolver
                .reject_awakeable(key, json!({ "winner": "reject" }), None)
                .await
        })
    };

    let resolved = resolved.await.expect("resolve task");
    let rejected = rejected.await.expect("reject task");

    assert!(
        matches!(
            (&resolved, &rejected),
            (Ok(()), Err(ResolveError::AlreadyResolved(_)))
                | (Err(ResolveError::AlreadyResolved(_)), Ok(()))
        ),
        "resolve/reject race should converge to one durable completion and one AlreadyResolved loser"
    );

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(rows.len(), 1, "resolve/reject race should persist only one row");
    let envelope = StreamEnvelope::from_json(rows[0].clone())?;
    assert!(
        matches!(
            envelope.kind(),
            Some(AWAKEABLE_RESOLVED_KIND) | Some(AWAKEABLE_REJECTED_KIND)
        ),
        "winner must be either the canonical resolved or rejected envelope"
    );

    stream_server.shutdown().await;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RaceResolution {
    winner: String,
}

#[tokio::test]
async fn resolve_awakeable_cross_process_duplicates_are_semantic_noop_on_replay() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let key = CompletionKey::prompt(
        SessionId::from("session-race".to_string()),
        RequestId::from("request-race".to_string()),
    );

    let observed = race_until_duplicate_resolutions(&stream_server, &key, 16).await?;

    assert_eq!(
        observed.rows.len(),
        2,
        "DSV-01 regression: the stream-level race must preserve both completion envelopes once the duplicate case is hit"
    );
    assert_eq!(
        observed.waiter_value, observed.first_value,
        "DSV-01 FirstResolutionWins: live waiter must resolve to the first appended completion",
    );
    assert_ne!(
        observed.waiter_value, observed.second_value,
        "DSV-01 FirstResolutionWins: waiter must not observe the later duplicate payload",
    );

    let replayed = tokio::time::timeout(
        Duration::from_millis(250),
        awakeable_context(observed.stream_url.clone()).awakeable::<RaceResolution>(key.clone()),
    )
    .await
    .context("timed out replaying already-resolved awakeable after duplicate race")??;

    assert_eq!(
        replayed, observed.first_value,
        "DSV-01 + DSV-02: replay must converge to the same first resolution even when a second duplicate append exists in the log",
    );

    stream_server.shutdown().await;
    Ok(())
}

struct DuplicateResolutionObservation {
    stream_url: String,
    rows: Vec<Value>,
    first_value: RaceResolution,
    second_value: RaceResolution,
    waiter_value: RaceResolution,
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

fn awakeable_context(stream_url: String) -> WorkflowContext {
    let mut subscriber_driver = DurableSubscriberDriver::new();
    subscriber_driver.register_passive(AwakeableSubscriber::new());
    WorkflowContext::with_subscriber_driver(stream_url, Arc::new(subscriber_driver))
}

async fn race_until_duplicate_resolutions(
    stream_server: &stream_server::TestStreamServer,
    key: &CompletionKey,
    max_attempts: usize,
) -> Result<DuplicateResolutionObservation> {
    for attempt in 1..=max_attempts {
        let stream_url = stream_server.stream_url(&format!("awakeable-race-{}-{attempt}", Uuid::new_v4()));
        ensure_json_stream_exists(&stream_url).await?;

        let resolver_a =
            AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "awakeable-race-a"));
        let resolver_b =
            AwakeableResolver::new(stream_url.clone(), json_producer(&stream_url, "awakeable-race-b"));

        let context = awakeable_context(stream_url.clone());
        let waiter = tokio::spawn({
            let key = key.clone();
            async move { context.awakeable::<RaceResolution>(key).await }
        });

        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let value_a = RaceResolution {
            winner: "resolver-a".to_string(),
        };
        let value_b = RaceResolution {
            winner: "resolver-b".to_string(),
        };

        let resolve_a = tokio::spawn({
            let barrier = Arc::clone(&barrier);
            let resolver = resolver_a.clone();
            let key = key.clone();
            let value = value_a.clone();
            async move {
                barrier.wait().await;
                resolver.resolve_awakeable(key, value, None).await
            }
        });
        let resolve_b = tokio::spawn({
            let barrier = Arc::clone(&barrier);
            let resolver = resolver_b.clone();
            let key = key.clone();
            let value = value_b.clone();
            async move {
                barrier.wait().await;
                resolver.resolve_awakeable(key, value, None).await
            }
        });

        barrier.wait().await;
        let (result_a, result_b) = tokio::join!(resolve_a, resolve_b);
        let result_a = result_a.context("resolver A panicked")?;
        let result_b = result_b.context("resolver B panicked")?;
        let waiter_value = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .with_context(|| format!("timed out waiting for awakeable race completion on attempt {attempt}"))?
            .context("awakeable race waiter panicked")??;

        let rows = read_all_rows(&stream_url).await?;
        let matching = parse_resolution_values(&rows, key)?;
        if matching.len() == 2 {
            assert!(
                matches!(
                    (&result_a, &result_b),
                    (Ok(()), Ok(()))
                        | (Ok(()), Err(ResolveError::AlreadyResolved(_)))
                        | (Err(ResolveError::AlreadyResolved(_)), Ok(()))
                ),
                "duplicate race attempt should resolve or observe an existing completion without transport errors"
            );

            return Ok(DuplicateResolutionObservation {
                stream_url,
                rows,
                first_value: matching[0].clone(),
                second_value: matching[1].clone(),
                waiter_value,
            });
        }
    }

    Err(anyhow::anyhow!(
        "did not observe duplicate awakeable_resolved appends after {max_attempts} attempts"
    ))
}

fn parse_resolution_values(rows: &[Value], key: &CompletionKey) -> Result<Vec<RaceResolution>> {
    rows.iter()
        .map(|row| StreamEnvelope::from_json(row.clone()).context("decode awakeable race envelope"))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter(|envelope| {
            envelope.kind() == Some(AWAKEABLE_RESOLVED_KIND)
                && envelope.completion_key().as_ref() == Some(key)
        })
        .map(|envelope| {
            envelope
                .value_as::<AwakeableResolved<RaceResolution>>()
                .map(|resolved| resolved.value)
                .ok_or_else(|| anyhow::anyhow!("decode duplicate awakeable_resolved payload"))
        })
        .collect()
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
