use durable_streams::Client as DurableStreamsClient;
use serde::Serialize;

use crate::{PersistedRuntimeSpec, RuntimeDescriptor};

#[derive(Debug, Clone, Serialize)]
struct StateHeaders {
    operation: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct StateEnvelope<T> {
    #[serde(rename = "type")]
    entity_type: &'static str,
    key: String,
    headers: StateHeaders,
    value: T,
}

pub async fn emit_runtime_spec_persisted(
    state_stream_url: &str,
    spec: &PersistedRuntimeSpec,
) -> anyhow::Result<()> {
    if state_stream_url.is_empty() {
        return Ok(());
    }

    let client = DurableStreamsClient::new();
    let mut stream = client.stream(state_stream_url);
    stream.set_content_type("application/json");
    let producer = stream
        .producer(format!("runtime-spec-{}", spec.runtime_key))
        .content_type("application/json")
        .build();
    producer.append_json(&StateEnvelope {
        entity_type: "runtime_spec",
        key: spec.runtime_key.clone(),
        headers: StateHeaders {
            operation: "insert",
        },
        value: spec,
    });
    producer.flush().await?;
    Ok(())
}

pub async fn emit_runtime_endpoints_persisted(
    state_stream_url: &str,
    descriptor: &RuntimeDescriptor,
) -> anyhow::Result<()> {
    if state_stream_url.is_empty() {
        return Ok(());
    }

    let client = DurableStreamsClient::new();
    let mut stream = client.stream(state_stream_url);
    stream.set_content_type("application/json");
    let producer = stream
        .producer(format!(
            "runtime-endpoints-{}-{}",
            descriptor.runtime_key,
            uuid::Uuid::new_v4()
        ))
        .content_type("application/json")
        .build();
    producer.append_json(&StateEnvelope {
        entity_type: "runtime_endpoints",
        key: descriptor.runtime_key.clone(),
        headers: StateHeaders {
            operation: "update",
        },
        value: descriptor,
    });
    producer.flush().await?;
    Ok(())
}
