use std::time::Duration;

use anyhow::{Context, Result};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, LiveMode, Offset, Producer};
use fireline_harness::{
    AWAKEABLE_RESOLVED_KIND, AWAKEABLE_WAITING_KIND, AwakeableResolver, CompletionKey,
    ResolveError, StreamEnvelope, TraceContext, WorkflowContext, awakeable_waiting_envelope,
};
use sacp::schema::{RequestId, SessionId};
use serde_json::{Value, json};
use uuid::Uuid;

#[path = "support/stream_server.rs"]
mod stream_server;

#[tokio::test]
async fn replay_returns_resolved_awakeable_without_resubscription() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("durable-promises-replay-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let resolver = AwakeableResolver::new(
        stream_url.clone(),
        json_producer(&stream_url, "durable-promises-replay"),
    );
    let key = prompt_key("replay-resolved");

    resolver
        .resolve_awakeable(key.clone(), json!({ "approved": true }), None)
        .await
        .context("seed already-resolved awakeable")?;

    let resolved = tokio::time::timeout(
        Duration::from_millis(250),
        WorkflowContext::new(stream_url.clone()).awakeable::<Value>(key.clone()),
    )
    .await
    .context("DSV-dp-40 replay should resolve from the existing durable completion")??;

    assert_eq!(
        resolved.get("approved").and_then(Value::as_bool),
        Some(true),
        "DSV-dp-40: replayed awakeable should surface the already-durable completion value",
    );

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_WAITING_KIND)?,
        0,
        "DSV-dp-40: replaying an already-resolved awakeable must not append a second waiting row",
    );
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_RESOLVED_KIND)?,
        1,
        "DSV-dp-40: one terminal completion row should remain the sole durable winner",
    );

    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn replay_rebinds_pending_awakeable_to_live_subscriber() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("durable-promises-replay-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let key = prompt_key("replay-pending");

    append_waiting(&stream_url, &key, None).await?;

    let waiter = tokio::spawn({
        let stream_url = stream_url.clone();
        let key = key.clone();
        async move { WorkflowContext::new(stream_url).awakeable::<bool>(key).await }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    AwakeableResolver::new(
        stream_url.clone(),
        json_producer(&stream_url, "durable-promises-replay"),
    )
    .resolve_awakeable(key.clone(), true, None)
    .await
    .context("resolve pending replayed awakeable")?;

    let resolved = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .context("DSV-dp-41 replayed pending awakeable should bind back to the live subscriber path")?
        .context("awakeable waiter panicked")??;
    assert!(
        resolved,
        "DSV-dp-41: replayed pending awakeable should resolve once the live completion arrives",
    );

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_WAITING_KIND)?,
        1,
        "DSV-dp-41: replay must reuse the existing waiting declaration instead of duplicating it",
    );
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_RESOLVED_KIND)?,
        1,
        "DSV-dp-41: the live subscriber path should produce one terminal completion",
    );

    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn resolution_during_rehydration_window_is_delivered_exactly_once() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("durable-promises-replay-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    append_padding_rows(&stream_url, 256).await?;
    let key = prompt_key("rehydration-window");

    append_waiting(&stream_url, &key, None).await?;

    let waiter = tokio::spawn({
        let stream_url = stream_url.clone();
        let key = key.clone();
        async move { WorkflowContext::new(stream_url).awakeable::<Value>(key).await }
    });

    let resolver = AwakeableResolver::new(
        stream_url.clone(),
        json_producer(&stream_url, "durable-promises-replay"),
    );
    tokio::time::sleep(Duration::from_millis(5)).await;
    resolver
        .resolve_awakeable(key.clone(), json!({ "winner": "rehydration" }), None)
        .await
        .context("resolve awakeable during replay rehydration window")?;

    let resolved = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .context("DSV-dp-42 replay/live race should still deliver exactly one completion")?
        .context("awakeable waiter panicked")??;
    assert_eq!(
        resolved.get("winner").and_then(Value::as_str),
        Some("rehydration"),
        "DSV-dp-42: the rehydrating waiter should observe the single durable winner",
    );

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_WAITING_KIND)?,
        1,
        "DSV-dp-42: replay should converge on the original waiting declaration",
    );
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_RESOLVED_KIND)?,
        1,
        "DSV-dp-42: replay/live interleaving must emit exactly one resolved completion",
    );

    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn concurrent_replay_and_live_resolve_do_not_double_resolve() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("durable-promises-replay-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let key = prompt_key("concurrent-replay-live");
    append_waiting(&stream_url, &key, None).await?;

    let waiter = tokio::spawn({
        let stream_url = stream_url.clone();
        let key = key.clone();
        async move { WorkflowContext::new(stream_url).awakeable::<Value>(key).await }
    });

    let resolver = AwakeableResolver::new(
        stream_url.clone(),
        json_producer(&stream_url, "durable-promises-replay"),
    );
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

    let first = first.await.context("join first replay/live resolver")?;
    let second = second.await.context("join second replay/live resolver")?;
    assert!(
        matches!(
            (&first, &second),
            (Ok(()), Err(ResolveError::AlreadyResolved(_)))
                | (Err(ResolveError::AlreadyResolved(_)), Ok(()))
        ),
        "DSV-dp-43: one replay/live resolver may win, but the loser must observe AlreadyResolved",
    );

    let resolved = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .context("DSV-dp-43 waiter should resolve despite concurrent replay/live resolution")?
        .context("awakeable waiter panicked")??;
    assert!(
        matches!(
            resolved.get("winner").and_then(Value::as_i64),
            Some(1 | 2)
        ),
        "DSV-dp-43: the waiter should observe exactly one durable winner payload",
    );

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_WAITING_KIND)?,
        1,
        "DSV-dp-43: replay/live convergence must not duplicate the waiting declaration",
    );
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_RESOLVED_KIND)?,
        1,
        "DSV-dp-43: concurrent replay/live resolution must persist one terminal completion row",
    );

    stream_server.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn traceparent_is_preserved_across_replay_boundary() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let stream_url = stream_server.stream_url(&format!("durable-promises-replay-{}", Uuid::new_v4()));
    ensure_json_stream_exists(&stream_url).await?;
    let key = prompt_key("traceparent-replay");
    let trace_context = TraceContext {
        traceparent: Some("00-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-bbbbbbbbbbbbbbbb-01".to_string()),
        tracestate: Some("vendor=value".to_string()),
        baggage: Some("scope=replay".to_string()),
    };

    append_waiting(&stream_url, &key, Some(trace_context.clone())).await?;

    let waiter = tokio::spawn({
        let stream_url = stream_url.clone();
        let key = key.clone();
        async move { WorkflowContext::new(stream_url).awakeable::<bool>(key).await }
    });

    AwakeableResolver::new(
        stream_url.clone(),
        json_producer(&stream_url, "durable-promises-replay"),
    )
    .resolve_awakeable(key.clone(), true, Some(trace_context.clone()))
    .await
    .context("resolve replayed awakeable with trace context")?;

    let resolved = tokio::time::timeout(Duration::from_secs(2), waiter)
        .await
        .context("DSV-dp-44 replay should preserve lineage while still resolving the waiter")?
        .context("awakeable waiter panicked")??;
    assert!(resolved, "DSV-dp-44: replayed waiter should still resolve successfully");

    let rows = read_all_rows(&stream_url).await?;
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_WAITING_KIND)?,
        1,
        "DSV-dp-44: replay should reuse the original traced waiting row instead of appending a new one",
    );
    assert_eq!(
        count_kind_for_key(&rows, &key, AWAKEABLE_RESOLVED_KIND)?,
        1,
        "DSV-dp-44: replay should keep one traced terminal completion row",
    );

    let waiting = row_for_kind(&rows, &key, AWAKEABLE_WAITING_KIND)?
        .expect("waiting row should exist");
    let resolved = row_for_kind(&rows, &key, AWAKEABLE_RESOLVED_KIND)?
        .expect("resolved row should exist");
    assert_eq!(traceparent(&waiting), trace_context.traceparent.as_deref());
    assert_eq!(traceparent(&resolved), trace_context.traceparent.as_deref());

    stream_server.shutdown().await;
    Ok(())
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

async fn append_waiting(
    stream_url: &str,
    key: &CompletionKey,
    trace_context: Option<TraceContext>,
) -> Result<()> {
    let mut envelope = awakeable_waiting_envelope(key.clone())?;
    if let Some(trace_context) = trace_context {
        envelope
            .value
            .as_mut()
            .and_then(Value::as_object_mut)
            .expect("waiting envelope should encode as JSON object")
            .insert("_meta".to_string(), Value::Object(trace_context.into_meta()));
    }
    let producer = json_producer(stream_url, "durable-promises-replay");
    producer.append_json(&envelope);
    producer
        .flush()
        .await
        .with_context(|| format!("flush awakeable waiting row to '{stream_url}'"))
}

async fn append_padding_rows(stream_url: &str, count: usize) -> Result<()> {
    let producer = json_producer(stream_url, "durable-promises-padding");
    for index in 0..count {
        producer.append_json(&json!({
            "type": "fixture",
            "key": format!("padding:{index}"),
            "headers": { "operation": "insert" },
            "value": { "kind": "padding", "index": index }
        }));
    }
    producer
        .flush()
        .await
        .with_context(|| format!("flush replay padding rows to '{stream_url}'"))
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
    while let Some(chunk) = reader
        .next_chunk()
        .await
        .with_context(|| format!("read durable stream '{stream_url}'"))?
    {
        if chunk.data.is_empty() {
            continue;
        }
        let batch: Vec<Value> =
            serde_json::from_slice(&chunk.data).context("decode durable stream JSON rows")?;
        rows.extend(batch);
    }

    Ok(rows)
}

fn count_kind_for_key(rows: &[Value], key: &CompletionKey, kind: &str) -> Result<usize> {
    Ok(rows
        .iter()
        .filter_map(|row| StreamEnvelope::from_json(row.clone()).ok())
        .filter(|envelope| {
            envelope.kind() == Some(kind) && envelope.completion_key().as_ref() == Some(key)
        })
        .count())
}

fn row_for_kind(rows: &[Value], key: &CompletionKey, kind: &str) -> Result<Option<StreamEnvelope>> {
    Ok(rows
        .iter()
        .filter_map(|row| StreamEnvelope::from_json(row.clone()).ok())
        .find(|envelope| {
            envelope.kind() == Some(kind) && envelope.completion_key().as_ref() == Some(key)
        }))
}

fn traceparent(envelope: &StreamEnvelope) -> Option<&str> {
    envelope
        .value
        .as_ref()
        .and_then(|value| value.get("_meta"))
        .and_then(|meta| meta.get("traceparent"))
        .and_then(Value::as_str)
}

fn prompt_key(suffix: &str) -> CompletionKey {
    CompletionKey::prompt(
        SessionId::from(format!("session-{suffix}")),
        RequestId::from(format!("request-{suffix}")),
    )
}
