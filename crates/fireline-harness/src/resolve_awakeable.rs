use std::fmt;
use std::sync::Arc;

use anyhow::{Context, Result};
use durable_streams::{Client as DurableStreamsClient, LiveMode, Offset, Producer};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::awakeable::{
    AWAKEABLE_REJECTED_KIND, AWAKEABLE_RESOLVED_KIND, AwakeableKey, awakeable_rejection_envelope,
    awakeable_resolution_envelope,
};
use crate::durable_subscriber::{StreamEnvelope, TraceContext};

/// Thin imperative resolver for passive awakeables.
///
/// The durable stream remains the sole source of truth for whether a key has
/// already been resolved. The local mutex only serializes in-process writers so
/// concurrent callers do not append duplicate completions within one process.
#[derive(Clone)]
pub struct AwakeableResolver {
    state_stream_url: String,
    producer: Producer,
    resolve_guard: Arc<Mutex<()>>,
}

impl AwakeableResolver {
    #[must_use]
    pub fn new(state_stream_url: impl Into<String>, producer: Producer) -> Self {
        Self {
            state_stream_url: state_stream_url.into(),
            producer,
            resolve_guard: Arc::new(Mutex::new(())),
        }
    }

    #[must_use]
    pub fn state_stream_url(&self) -> &str {
        &self.state_stream_url
    }

    pub async fn resolve_awakeable<T>(
        &self,
        key: AwakeableKey,
        value: T,
        trace_context: Option<TraceContext>,
    ) -> Result<(), ResolveError>
    where
        T: Serialize,
    {
        let _guard = self.resolve_guard.lock().await;
        if self
            .has_resolution(&key)
            .await
            .map_err(ResolveError::Stream)?
        {
            return Err(ResolveError::AlreadyResolved(key));
        }

        let envelope = resolution_envelope(key.clone(), value, trace_context)
            .map_err(ResolveError::Serialize)?;
        self.producer.append_json(&envelope);
        self.producer
            .flush()
            .await
            .with_context(|| format!("flush awakeable resolution '{}'", key.storage_key()))
            .map_err(ResolveError::Stream)?;
        Ok(())
    }

    pub async fn resolve<T>(
        &self,
        key: AwakeableKey,
        value: T,
        trace_context: Option<TraceContext>,
    ) -> Result<(), ResolveError>
    where
        T: Serialize,
    {
        self.resolve_awakeable(key, value, trace_context).await
    }

    pub async fn reject_awakeable<E>(
        &self,
        key: AwakeableKey,
        error: E,
        trace_context: Option<TraceContext>,
    ) -> Result<(), ResolveError>
    where
        E: Serialize,
    {
        let _guard = self.resolve_guard.lock().await;
        if self
            .has_resolution(&key)
            .await
            .map_err(ResolveError::Stream)?
        {
            return Err(ResolveError::AlreadyResolved(key));
        }

        let envelope = rejection_envelope(key.clone(), error, trace_context)
            .map_err(ResolveError::Serialize)?;
        self.producer.append_json(&envelope);
        self.producer
            .flush()
            .await
            .with_context(|| format!("flush awakeable rejection '{}'", key.storage_key()))
            .map_err(ResolveError::Stream)?;
        Ok(())
    }

    async fn has_resolution(&self, key: &AwakeableKey) -> Result<bool> {
        let client = DurableStreamsClient::new();
        let stream = client.stream(&self.state_stream_url);
        let mut reader = stream
            .read()
            .offset(Offset::Beginning)
            .live(LiveMode::Off)
            .build()
            .with_context(|| {
                format!(
                    "build awakeable resolution reader for '{}'",
                    self.state_stream_url
                )
            })?;

        loop {
            let Some(chunk) = reader
                .next_chunk()
                .await
                .with_context(|| format!("read awakeable stream '{}'", self.state_stream_url))?
            else {
                return Ok(false);
            };

            if !chunk.data.is_empty() {
                let rows: Vec<Value> =
                    serde_json::from_slice(&chunk.data).context("decode awakeable stream rows")?;
                for row in rows {
                    let envelope =
                        StreamEnvelope::from_json(row).context("decode awakeable stream envelope")?;
                    if (envelope.kind() == Some(AWAKEABLE_RESOLVED_KIND)
                        || envelope.kind() == Some(AWAKEABLE_REJECTED_KIND))
                        && envelope.completion_key().as_ref() == Some(key)
                    {
                        return Ok(true);
                    }
                }
            }

            if chunk.up_to_date {
                return Ok(false);
            }
        }
    }
}

fn resolution_envelope<T>(
    key: AwakeableKey,
    value: T,
    trace_context: Option<TraceContext>,
) -> Result<StreamEnvelope>
where
    T: Serialize,
{
    let mut envelope = awakeable_resolution_envelope(key, value)?;
    let Some(trace_context) = trace_context else {
        return Ok(envelope);
    };
    if trace_context.is_empty() {
        return Ok(envelope);
    }

    let value = envelope
        .value
        .as_mut()
        .and_then(Value::as_object_mut)
        .context("awakeable resolution envelope missing JSON object payload")?;
    value.insert("_meta".to_string(), Value::Object(trace_context.into_meta()));
    Ok(envelope)
}

fn rejection_envelope<E>(
    key: AwakeableKey,
    error: E,
    trace_context: Option<TraceContext>,
) -> Result<StreamEnvelope>
where
    E: Serialize,
{
    let mut envelope = awakeable_rejection_envelope(key, error)?;
    let Some(trace_context) = trace_context else {
        return Ok(envelope);
    };
    if trace_context.is_empty() {
        return Ok(envelope);
    }

    let value = envelope
        .value
        .as_mut()
        .and_then(Value::as_object_mut)
        .context("awakeable rejection envelope missing JSON object payload")?;
    value.insert("_meta".to_string(), Value::Object(trace_context.into_meta()));
    Ok(envelope)
}

#[derive(Debug)]
pub enum ResolveError {
    AlreadyResolved(AwakeableKey),
    Serialize(anyhow::Error),
    Stream(anyhow::Error),
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyResolved(key) => {
                write!(f, "awakeable '{}' is already resolved", key.storage_key())
            }
            Self::Serialize(error) => write!(f, "serialize awakeable resolution: {error}"),
            Self::Stream(error) => write!(f, "persist awakeable resolution: {error}"),
        }
    }
}

impl std::error::Error for ResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AlreadyResolved(_) => None,
            Self::Serialize(error) | Self::Stream(error) => Some(error.root_cause()),
        }
    }
}
