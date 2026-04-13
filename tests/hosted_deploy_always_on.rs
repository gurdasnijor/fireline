use std::net::IpAddr;
use std::time::Duration;

use anyhow::Result;
use durable_streams::{Client as DsClient, Producer};
use fireline_harness::{
    DeploymentWakeRequested, WakeTimerRequest, emit_host_endpoints_persisted,
    wake_timer_request_envelope,
};
use fireline_host::bootstrap::{BootstrapConfig, BootstrapHandle, start};
use fireline_session::{
    Endpoint, HostDescriptor, HostStatus, SandboxProviderKind, TopologyComponentSpec, TopologySpec,
};
use sacp::schema::{RequestId, SessionId};
use uuid::Uuid;

#[path = "support/managed_agent_suite.rs"]
mod managed_agent_suite;
#[path = "support/stream_server.rs"]
mod stream_server;

#[tokio::test]
async fn hosted_boot_adds_always_on_deployment_by_default() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let host_key = format!("runtime:{}", Uuid::new_v4());
    let state_stream = format!("hosted-always-on-{}", Uuid::new_v4());
    let handle = start_host(
        &stream_server.base_url,
        host_key.clone(),
        state_stream.clone(),
        TopologySpec::default(),
        Some("http://127.0.0.1:65535".to_string()),
    )
    .await?;

    let result = async {
        persist_live_runtime_endpoints(&handle, &host_key).await?;
        let session_id = managed_agent_suite::create_session(&handle.acp_url).await?;
        let session_id_match = format!("\"sessionId\":\"{session_id}\"");
        append_json(
            &handle.state_stream_url,
            "hosted-always-on",
            &serde_json::json!({
                "type": "deployment",
                "key": format!("{session_id}:wake_requested"),
                "headers": { "operation": "insert" },
                "value": DeploymentWakeRequested::new(SessionId::from(session_id.clone())),
            }),
        )
        .await?;

        let body = managed_agent_suite::wait_for_stream_rows(
            &handle.state_stream_url,
            &["\"kind\":\"sandbox_provisioned\"", &session_id_match],
            Duration::from_secs(5),
        )
        .await?;

        assert!(
            body.contains("\"kind\":\"sandbox_provisioned\""),
            "hosted boot should register AlwaysOnDeploymentSubscriber by default: {body}"
        );
        Ok(())
    }
    .await;

    handle.shutdown().await?;
    stream_server.shutdown().await;
    result
}

#[tokio::test]
async fn hosted_boot_can_reference_wake_timer_subscriber() -> Result<()> {
    let stream_server = stream_server::TestStreamServer::spawn().await?;
    let host_key = format!("runtime:{}", Uuid::new_v4());
    let state_stream = format!("hosted-wake-timer-{}", Uuid::new_v4());
    let handle = start_host(
        &stream_server.base_url,
        host_key,
        state_stream,
        TopologySpec {
            components: vec![
                TopologyComponentSpec {
                    name: "peer_mcp".to_string(),
                    config: None,
                },
                TopologyComponentSpec {
                    name: "wake_timer".to_string(),
                    config: None,
                },
            ],
        },
        Some("http://127.0.0.1:65535".to_string()),
    )
    .await?;

    let result = async {
        let session_id = managed_agent_suite::create_session(&handle.acp_url).await?;
        let session_id_match = format!("\"sessionId\":\"{session_id}\"");
        let request = WakeTimerRequest::new(
            SessionId::from(session_id.clone()),
            RequestId::from("wake-timer-test".to_string()),
            now_ms() + 25,
        );
        let envelope = wake_timer_request_envelope(request)?;
        append_json(&handle.state_stream_url, "hosted-wake-timer", &envelope).await?;

        let body = managed_agent_suite::wait_for_stream_rows(
            &handle.state_stream_url,
            &[
                "\"kind\":\"timer_fired\"",
                &session_id_match,
                "\"requestId\":\"wake-timer-test\"",
            ],
            Duration::from_secs(5),
        )
        .await?;

        assert!(
            body.contains("\"kind\":\"timer_fired\""),
            "hosted boot should start WakeTimerSubscriber when requested by topology: {body}"
        );
        Ok(())
    }
    .await;

    handle.shutdown().await?;
    stream_server.shutdown().await;
    result
}

async fn start_host(
    durable_streams_url: &str,
    host_key: String,
    state_stream: String,
    topology: TopologySpec,
    control_plane_url: Option<String>,
) -> Result<BootstrapHandle> {
    start(BootstrapConfig {
        host: "127.0.0.1".parse::<IpAddr>()?,
        port: 0,
        name: format!("hosted-{}", Uuid::new_v4()),
        host_key,
        node_id: "node:hosted-deploy".to_string(),
        agent_command: vec![managed_agent_suite::testy_bin().display().to_string()],
        mounted_resources: Vec::new(),
        state_stream: Some(state_stream),
        durable_streams_url: durable_streams_url.to_string(),
        peer_directory_path: managed_agent_suite::temp_path("fireline-hosted-deploy-peers"),
        control_plane_url,
        topology,
    })
    .await
}

async fn persist_live_runtime_endpoints(handle: &BootstrapHandle, host_key: &str) -> Result<()> {
    emit_host_endpoints_persisted(
        &handle.state_stream_url,
        &HostDescriptor {
            host_key: host_key.to_string(),
            host_id: handle.host_id.clone(),
            node_id: "node:hosted-deploy".to_string(),
            provider: SandboxProviderKind::Local,
            provider_instance_id: handle.host_id.clone(),
            status: HostStatus::Ready,
            acp: Endpoint::new(handle.acp_url.clone()),
            state: Endpoint::new(handle.state_stream_url.clone()),
            helper_api_base_url: None,
            created_at_ms: now_ms(),
            updated_at_ms: now_ms(),
        },
    )
    .await
}

async fn append_json(
    stream_url: &str,
    producer_id: &str,
    value: &impl serde::Serialize,
) -> Result<()> {
    let producer = json_producer(stream_url, producer_id);
    producer.append_json(value);
    producer.flush().await?;
    Ok(())
}

fn json_producer(stream_url: &str, producer_id: &str) -> Producer {
    let client = DsClient::new();
    let mut stream = client.stream(stream_url);
    stream.set_content_type("application/json");
    stream
        .producer(producer_id.to_string())
        .content_type("application/json")
        .build()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
