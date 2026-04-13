use anyhow::{Context, Result};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, LiveMode, Offset, Producer};
use fireline_harness::{
    AWAKEABLE_RESOLVED_KIND, AwakeableResolver, CompletionKey, ResolveError, StreamEnvelope,
    TraceContext,
};
use sacp::schema::{RequestId, SessionId, ToolCallId};
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
