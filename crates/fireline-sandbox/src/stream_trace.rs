use durable_streams::Client as DurableStreamsClient;
use serde::Serialize;

use crate::{PersistedHostSpec, HostDescriptor};

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

pub async fn emit_host_spec_persisted(
    state_stream_url: &str,
    spec: &PersistedHostSpec,
) -> anyhow::Result<()> {
    if state_stream_url.is_empty() {
        return Ok(());
    }

    let client = DurableStreamsClient::new();
    let mut stream = client.stream(state_stream_url);
    stream.set_content_type("application/json");
    let producer = stream
        .producer(format!("runtime-spec-{}", spec.host_key))
        .content_type("application/json")
        .build();
    producer.append_json(&StateEnvelope {
        entity_type: "runtime_spec",
        key: spec.host_key.clone(),
        headers: StateHeaders {
            operation: "insert",
        },
        value: spec,
    });
    producer.flush().await?;
    Ok(())
}

pub async fn emit_host_endpoints_persisted(
    state_stream_url: &str,
    descriptor: &HostDescriptor,
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
            descriptor.host_key,
            uuid::Uuid::new_v4()
        ))
        .content_type("application/json")
        .build();
    producer.append_json(&StateEnvelope {
        entity_type: "runtime_endpoints",
        key: descriptor.host_key.clone(),
        headers: StateHeaders {
            operation: "update",
        },
        value: descriptor,
    });
    producer.flush().await?;
    Ok(())
}
