use std::sync::Arc;

use anyhow::{Context, Result};
use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset, Producer};
use serde_json::Value;
use tokio::task::JoinHandle;

use crate::{ActiveSubscriber, HandlerOutcome, StreamEnvelope};

pub fn spawn_active_subscriber_task<S, F>(
    task_name: &'static str,
    subscriber: S,
    state_stream_url: impl Into<String>,
    state_producer: Producer,
    completion_to_envelope: F,
) -> JoinHandle<()>
where
    S: ActiveSubscriber + 'static,
    F: Fn(S::Completion) -> Result<StreamEnvelope> + Send + Sync + 'static,
{
    let state_stream_url = state_stream_url.into();
    tokio::spawn(async move {
        if let Err(error) = run_active_subscriber_task(
            task_name,
            Arc::new(subscriber),
            state_stream_url,
            state_producer,
            Arc::new(completion_to_envelope),
        )
        .await
        {
            tracing::warn!(task = task_name, %error, "active durable subscriber task stopped");
        }
    })
}

async fn run_active_subscriber_task<S, F>(
    task_name: &'static str,
    subscriber: Arc<S>,
    state_stream_url: String,
    state_producer: Producer,
    completion_to_envelope: Arc<F>,
) -> Result<()>
where
    S: ActiveSubscriber + 'static,
    F: Fn(S::Completion) -> Result<StreamEnvelope> + Send + Sync + 'static,
{
    let client = DurableStreamsClient::new();
    let stream = client.stream(&state_stream_url);
    let mut reader = stream
        .read()
        .offset(Offset::Beginning)
        .live(LiveMode::Sse)
        .build()
        .with_context(|| format!("build active subscriber reader for '{state_stream_url}'"))?;
    let mut log: Vec<StreamEnvelope> = Vec::new();

    while let Some(chunk) = reader
        .next_chunk()
        .await
        .with_context(|| format!("read active subscriber stream '{state_stream_url}'"))?
    {
        if chunk.data.is_empty() {
            continue;
        }

        let mut chunk_log = Vec::new();
        let events: Vec<Value> =
            serde_json::from_slice(&chunk.data).context("parse active subscriber chunk")?;
        for event in events {
            match StreamEnvelope::from_json(event) {
                Ok(envelope) => chunk_log.push(envelope),
                Err(error) => {
                    tracing::warn!(task = task_name, %error, "skipped malformed stream envelope");
                }
            }
        }
        log.extend(chunk_log.iter().cloned());

        for envelope in chunk_log {
            let Some(event) = subscriber.matches(&envelope) else {
                continue;
            };
            if subscriber.is_completed(&event, &log) {
                continue;
            }

            match subscriber.handle(event).await {
                HandlerOutcome::Completed(completion) => {
                    let completion_envelope = completion_to_envelope(completion)?;
                    state_producer.append_json(&completion_envelope);
                    state_producer
                        .flush()
                        .await
                        .context("flush active subscriber completion")?;
                    log.push(completion_envelope);
                }
                HandlerOutcome::RetryTransient(error) => {
                    tracing::warn!(task = task_name, %error, "active subscriber transient failure");
                }
                HandlerOutcome::Failed(error) => {
                    tracing::warn!(task = task_name, %error, "active subscriber terminal failure");
                }
            }
        }
    }

    Ok(())
}
