use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fireline_session::{
    HostDescriptor, HostIndex, HostStatus, PersistedHostSpec, SessionIndex, StateMaterializer,
};
use reqwest::Client as HttpClient;
use tracing::{info, instrument};

pub mod load_coordinator;
pub mod primitive;

/// Resume a session by session id.
///
/// The caller must supply the **shared** state stream URL explicitly.
/// Earlier versions of this helper discovered the shared stream URL by
/// listing the control plane's runtimes and using the first non-empty
/// `state.url` field — that was a hack that depended on at least one
/// runtime already existing and on its descriptor carrying a URL that
/// matched the shared stream. Resume's contract is "given a shared
/// state endpoint, look up this session and hand the caller a Ready
/// runtime for it", and the shared endpoint is best modeled as an
/// explicit parameter the caller already holds (e.g., the control
/// plane's configured `shared_stream_base_url/{stream_name}`) — not
/// as something to rediscover from live runtime state every call.
#[instrument(skip(http), fields(session_id, control_plane_url, shared_state_url))]
pub async fn resume(
    http: &HttpClient,
    control_plane_url: &str,
    shared_state_url: &str,
    session_id: &str,
) -> Result<HostDescriptor> {
    let started = tokio::time::Instant::now();
    let (_, shared_hosts) = wait_for_shared_state_indexes(shared_state_url, session_id).await?;
    let runtime =
        lookup_runtime_for_session(http, control_plane_url, &shared_hosts, shared_state_url)
            .await?;

    if runtime.status == HostStatus::Ready {
        info!(
            session_id,
            host_key = runtime.host_key,
            elapsed_ms = started.elapsed().as_millis(),
            "resume found live ready runtime"
        );
        return Ok(runtime);
    }

    let persisted =
        host_spec_for_state_stream(&shared_hosts, shared_state_url).await.ok_or_else(|| {
            anyhow!("host_spec for session '{session_id}' not found in the shared host index")
        })?;
    let created = http
        .post(format!(
            "{}/v1/runtimes",
            control_plane_url.trim_end_matches('/')
        ))
        .json(&persisted.create_spec)
        .send()
        .await
        .context("create runtime during resume")?
        .error_for_status()
        .context("control plane rejected resume create")?
        .json::<HostDescriptor>()
        .await
        .context("decode resumed runtime descriptor")?;

    let ready = wait_for_runtime_ready(http, control_plane_url, &created.host_key).await?;
    info!(
        session_id,
        host_key = ready.host_key,
        elapsed_ms = started.elapsed().as_millis(),
        "resume recreated runtime from persisted spec"
    );
    Ok(ready)
}

#[instrument(fields(host_key, state_stream_url))]
pub async fn reconstruct_host_spec_from_log(
    state_stream_url: &str,
    host_key: &str,
) -> Result<PersistedHostSpec> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut attempts = 0usize;
    loop {
        attempts += 1;
        let (_, host_index) = materialize_state_indexes(state_stream_url).await?;
        if let Some(spec) = host_index.spec_for(host_key).await {
            info!(
                host_key,
                attempts, "reconstructed host_spec from durable state"
            );
            return Ok(spec);
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "host_spec '{host_key}' not found in state stream"
            ));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[instrument(fields(state_stream_url))]
pub async fn materialize_state_indexes(state_stream_url: &str) -> Result<(SessionIndex, HostIndex)> {
    let index = SessionIndex::new();
    let host_index = HostIndex::new();
    let materializer =
        StateMaterializer::new(vec![Arc::new(index.clone()), Arc::new(host_index.clone())]);
    let task = materializer.connect(state_stream_url.to_string());
    task.preload().await?;
    task.abort();
    Ok((index, host_index))
}

pub async fn materialize_session_index(state_stream_url: &str) -> Result<SessionIndex> {
    materialize_state_indexes(state_stream_url)
        .await
        .map(|(sessions, _)| sessions)
}

pub async fn materialize_shared_session_index(shared_state_url: &str) -> Result<SessionIndex> {
    materialize_session_index(shared_state_url).await
}

pub async fn materialize_shared_host_index(shared_state_url: &str) -> Result<HostIndex> {
    materialize_state_indexes(shared_state_url)
        .await
        .map(|(_, hosts)| hosts)
}

#[instrument(fields(session_id, shared_state_url))]
async fn wait_for_shared_state_indexes(
    shared_state_url: &str,
    session_id: &str,
) -> Result<(SessionIndex, HostIndex)> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut attempts = 0usize;
    loop {
        attempts += 1;
        let (session_index, host_index) = materialize_state_indexes(shared_state_url).await?;
        if session_index.get(session_id).await.is_some()
            && host_spec_for_state_stream(&host_index, shared_state_url)
                .await
                .is_some()
        {
            info!(
                session_id,
                attempts, "shared session index is ready for resume"
            );
            return Ok((session_index, host_index));
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "session '{session_id}' or its host_spec was not found in the shared session index at {shared_state_url}"
            ));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[instrument(skip(http, index), fields(control_plane_url, shared_state_url))]
async fn lookup_runtime_for_session(
    http: &HttpClient,
    control_plane_url: &str,
    index: &HostIndex,
    shared_state_url: &str,
) -> Result<HostDescriptor> {
    if let Some(descriptor) = index
        .list_endpoints()
        .await
        .into_iter()
        .find(|descriptor| descriptor.state.url == shared_state_url)
    {
        return Ok(descriptor);
    }

    let descriptors = http
        .get(format!("{}/v1/runtimes", control_plane_url.trim_end_matches('/')))
        .send()
        .await
        .context("list runtimes for shared state stream")?
        .error_for_status()
        .context("control plane rejected runtime list for shared state stream")?
        .json::<Vec<HostDescriptor>>()
        .await
        .context("decode control-plane runtime list for shared state stream")?;

    descriptors
        .into_iter()
        .find(|descriptor| descriptor.state.url == shared_state_url)
        .ok_or_else(|| anyhow!("no runtime found for shared state stream '{shared_state_url}'"))
}

async fn host_spec_for_state_stream(
    index: &HostIndex,
    shared_state_url: &str,
) -> Option<PersistedHostSpec> {
    let state_stream = shared_state_url.rsplit('/').next()?;
    for host_key in index.known_host_keys().await {
        if let Some(spec) = index.spec_for(&host_key).await {
            if spec.create_spec.state_stream.as_deref() == Some(state_stream) {
                return Some(spec);
            }
        }
    }
    None
}

#[instrument(skip(http), fields(host_key, control_plane_url))]
async fn wait_for_runtime_ready(
    http: &HttpClient,
    control_plane_url: &str,
    host_key: &str,
) -> Result<HostDescriptor> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut polls = 0usize;
    loop {
        polls += 1;
        let descriptor = http
            .get(format!(
                "{}/v1/runtimes/{}",
                control_plane_url.trim_end_matches('/'),
                host_key
            ))
            .send()
            .await
            .context("fetch runtime during resume")?
            .error_for_status()
            .context("control plane rejected runtime fetch")?
            .json::<HostDescriptor>()
            .await
            .context("decode control-plane runtime descriptor")?;

        if descriptor.status == HostStatus::Ready {
            info!(host_key, polls, "runtime became ready during resume");
            return Ok(descriptor);
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for runtime '{host_key}' to become ready during resume"
            ));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
