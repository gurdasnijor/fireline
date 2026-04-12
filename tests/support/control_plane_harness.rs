use std::time::Duration;

use anyhow::{Result, anyhow};
use durable_streams::{Client as DsClient, Offset};
use fireline_sandbox::{SandboxDescriptor, SandboxStatus};
use fireline_session::{
    HostDescriptor, HostInstanceRecord, HostInstanceStatus, HostStatus, PersistedHostSpec,
    SandboxProviderKind, StateEnvelope,
};
use serde_json::Value;

pub(crate) async fn wait_for_host_status(
    base_url: &str,
    sandbox_id: &str,
    expected: HostStatus,
) -> Result<HostDescriptor> {
    let client = reqwest::Client::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let response = client
            .get(format!("{base_url}/v1/sandboxes/{sandbox_id}"))
            .send()
            .await?;
        if response.status().is_success() {
            let sandbox = response.json::<SandboxDescriptor>().await?;
            if host_status_from_sandbox_status(sandbox.status.clone()) == expected {
                if let Some(descriptor) =
                    wait_for_host_descriptor(&sandbox, expected, Duration::from_secs(1)).await?
                {
                    return Ok(descriptor);
                }
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for sandbox '{sandbox_id}' to become '{expected:?}'"
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_for_host_descriptor(
    sandbox: &SandboxDescriptor,
    expected: HostStatus,
    timeout: Duration,
) -> Result<Option<HostDescriptor>> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(descriptor) =
            host_descriptor_from_sandbox(sandbox, Duration::from_millis(250)).await?
        {
            if descriptor.status == expected {
                return Ok(Some(descriptor));
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn host_descriptor_from_sandbox(
    sandbox: &SandboxDescriptor,
    timeout: Duration,
) -> Result<Option<HostDescriptor>> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let specs = read_persisted_host_specs(&sandbox.state.url).await?;
        if let Some(spec) = specs.into_iter().find(|spec| spec.host_key == sandbox.id) {
            let instances = read_runtime_instances(&sandbox.state.url).await?;
            if let Some(instance) = select_runtime_instance(&spec, sandbox, &instances) {
                return Ok(Some(HostDescriptor {
                    host_key: sandbox.id.clone(),
                    host_id: instance.instance_id.clone(),
                    node_id: spec.node_id.clone(),
                    provider: sandbox_provider_kind_from_name(&sandbox.provider),
                    provider_instance_id: instance.instance_id.clone(),
                    status: host_status_from_sandbox_status(sandbox.status.clone()),
                    acp: sandbox.acp.clone(),
                    state: sandbox.state.clone(),
                    helper_api_base_url: None,
                    created_at_ms: sandbox.created_at_ms,
                    updated_at_ms: sandbox.updated_at_ms,
                }));
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn read_persisted_host_specs(state_stream_url: &str) -> Result<Vec<PersistedHostSpec>> {
    let envelopes = read_all_events(state_stream_url).await?;
    Ok(envelopes
        .iter()
        .filter(|env| env.entity_type() == Some("runtime_spec"))
        .filter_map(|env| env.value.clone())
        .filter_map(|value| serde_json::from_value::<PersistedHostSpec>(value).ok())
        .collect())
}

async fn read_runtime_instances(state_stream_url: &str) -> Result<Vec<HostInstanceRecord>> {
    let envelopes = read_all_events(state_stream_url).await?;
    Ok(envelopes
        .iter()
        .filter(|env| env.entity_type() == Some("runtime_instance"))
        .filter_map(|env| env.value.clone())
        .filter_map(|value| serde_json::from_value::<HostInstanceRecord>(value).ok())
        .collect())
}

async fn read_all_events(state_stream_url: &str) -> Result<Vec<StateEnvelope>> {
    Ok(
        parse_state_events(&read_state_stream(state_stream_url).await?)
            .into_iter()
            .filter_map(|event| serde_json::from_value::<StateEnvelope>(event).ok())
            .collect(),
    )
}

async fn read_state_stream(state_stream_url: &str) -> Result<String> {
    let client = DsClient::new();
    let stream = client.stream(state_stream_url);
    let mut reader = stream.read().offset(Offset::Beginning).build()?;
    let mut body = String::new();
    while let Some(chunk) = reader.next_chunk().await? {
        body.push_str(std::str::from_utf8(&chunk.data)?);
        if chunk.up_to_date {
            break;
        }
    }
    Ok(body)
}

fn select_runtime_instance<'a>(
    spec: &PersistedHostSpec,
    sandbox: &SandboxDescriptor,
    instances: &'a [HostInstanceRecord],
) -> Option<&'a HostInstanceRecord> {
    let desired = desired_runtime_instance_status(&sandbox.status);
    let mut matches: Vec<&HostInstanceRecord> = instances
        .iter()
        .filter(|instance| instance.host_name == spec.create_spec.name)
        .collect();

    if let Some(desired) = desired {
        let mut status_matches: Vec<&HostInstanceRecord> = matches
            .iter()
            .copied()
            .filter(|instance| instance.status == desired)
            .collect();
        status_matches.sort_by_key(|instance| (instance.updated_at, instance.created_at));
        if let Some(found) = status_matches.into_iter().last() {
            return Some(found);
        }
    }

    matches.sort_by_key(|instance| (instance.updated_at, instance.created_at));
    matches.into_iter().last()
}

fn desired_runtime_instance_status(status: &SandboxStatus) -> Option<HostInstanceStatus> {
    match status {
        SandboxStatus::Creating
        | SandboxStatus::Ready
        | SandboxStatus::Busy
        | SandboxStatus::Idle => Some(HostInstanceStatus::Running),
        SandboxStatus::Stopped | SandboxStatus::Broken => Some(HostInstanceStatus::Stopped),
    }
}

fn host_status_from_sandbox_status(status: SandboxStatus) -> HostStatus {
    match status {
        SandboxStatus::Creating => HostStatus::Starting,
        SandboxStatus::Ready => HostStatus::Ready,
        SandboxStatus::Busy => HostStatus::Busy,
        SandboxStatus::Idle => HostStatus::Idle,
        SandboxStatus::Stopped => HostStatus::Stopped,
        SandboxStatus::Broken => HostStatus::Broken,
    }
}

fn sandbox_provider_kind_from_name(name: &str) -> SandboxProviderKind {
    match name {
        "docker" => SandboxProviderKind::Docker,
        _ => SandboxProviderKind::Local,
    }
}

fn parse_state_events(body: &str) -> Vec<Value> {
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Array(events)) => events,
        Ok(value) => vec![value],
        Err(_) => {
            let mut stream = serde_json::Deserializer::from_str(body).into_iter::<Value>();
            std::iter::from_fn(move || stream.next())
                .filter_map(|result| result.ok())
                .collect()
        }
    }
}
