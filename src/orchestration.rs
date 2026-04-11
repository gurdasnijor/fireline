use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fireline_conductor::runtime::{PersistedRuntimeSpec, RuntimeDescriptor, RuntimeStatus};
use reqwest::Client as HttpClient;

use crate::runtime_materializer::RuntimeMaterializer;
use crate::session_index::SessionIndex;

pub async fn resume(
    http: &HttpClient,
    control_plane_url: &str,
    session_id: &str,
) -> Result<RuntimeDescriptor> {
    let shared_index =
        wait_for_shared_session_index(http, control_plane_url, session_id).await?;
    let runtime = lookup_runtime_for_session(http, control_plane_url, &shared_index, session_id)
        .await?;

    if runtime.status == RuntimeStatus::Ready {
        return Ok(runtime);
    }

    let persisted = shared_index
        .runtime_spec_for_session(session_id)
        .await
        .ok_or_else(|| {
            anyhow!("runtime_spec for session '{session_id}' not found in shared session index")
        })?;
    let created = http
        .post(format!("{}/v1/runtimes", control_plane_url.trim_end_matches('/')))
        .json(&persisted.create_spec)
        .send()
        .await
        .context("create runtime during resume")?
        .error_for_status()
        .context("control plane rejected resume create")?
        .json::<RuntimeDescriptor>()
        .await
        .context("decode resumed runtime descriptor")?;

    wait_for_runtime_ready(http, control_plane_url, &created.runtime_key).await
}

pub async fn reconstruct_runtime_spec_from_log(
    state_stream_url: &str,
    runtime_key: &str,
) -> Result<PersistedRuntimeSpec> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let index = materialize_session_index(state_stream_url).await?;
        if let Some(spec) = index.runtime_spec(runtime_key).await {
            return Ok(spec);
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "runtime_spec '{runtime_key}' not found in state stream"
            ));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub async fn materialize_session_index(state_stream_url: &str) -> Result<SessionIndex> {
    let index = SessionIndex::new();
    let materializer = RuntimeMaterializer::new(vec![Arc::new(index.clone())]);
    let task = materializer.connect(state_stream_url.to_string());
    task.preload().await?;
    task.abort();
    Ok(index)
}

pub async fn materialize_shared_session_index(
    http: &HttpClient,
    control_plane_url: &str,
) -> Result<SessionIndex> {
    let shared_state_stream_url = shared_state_stream_url(http, control_plane_url).await?;
    materialize_session_index(&shared_state_stream_url).await
}

async fn wait_for_shared_session_index(
    http: &HttpClient,
    control_plane_url: &str,
    session_id: &str,
) -> Result<SessionIndex> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let index = materialize_shared_session_index(http, control_plane_url).await?;
        if index.get(session_id).await.is_some()
            && index.runtime_spec_for_session(session_id).await.is_some()
        {
            return Ok(index);
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "session '{session_id}' or its runtime_spec was not found in the shared session index"
            ));
        }

        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn lookup_runtime_for_session(
    http: &HttpClient,
    control_plane_url: &str,
    index: &SessionIndex,
    session_id: &str,
) -> Result<RuntimeDescriptor> {
    let record = index
        .get(session_id)
        .await
        .ok_or_else(|| anyhow!("session '{session_id}' not found in shared session index"))?;

    http.get(format!(
        "{}/v1/runtimes/{}",
        control_plane_url.trim_end_matches('/'),
        record.runtime_key
    ))
    .send()
    .await
    .context("fetch runtime for session from control plane")?
    .error_for_status()
    .context("control plane rejected runtime lookup for session")?
    .json::<RuntimeDescriptor>()
    .await
    .context("decode control-plane runtime descriptor for session")
}

async fn list_runtimes(
    http: &HttpClient,
    control_plane_url: &str,
) -> Result<Vec<RuntimeDescriptor>> {
    http.get(format!("{}/v1/runtimes", control_plane_url.trim_end_matches('/')))
        .send()
        .await
        .context("list control-plane runtimes")?
        .error_for_status()
        .context("control plane rejected runtime list")?
        .json::<Vec<RuntimeDescriptor>>()
        .await
        .context("decode control-plane runtime list")
}

async fn shared_state_stream_url(http: &HttpClient, control_plane_url: &str) -> Result<String> {
    let runtimes = list_runtimes(http, control_plane_url).await?;
    runtimes
        .into_iter()
        .find_map(|runtime| (!runtime.state.url.is_empty()).then_some(runtime.state.url))
        .ok_or_else(|| anyhow!("no shared state stream URL available from control-plane runtimes"))
}

async fn wait_for_runtime_ready(
    http: &HttpClient,
    control_plane_url: &str,
    runtime_key: &str,
) -> Result<RuntimeDescriptor> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let descriptor = http
            .get(format!(
                "{}/v1/runtimes/{}",
                control_plane_url.trim_end_matches('/'),
                runtime_key
            ))
            .send()
            .await
            .context("fetch runtime during resume")?
            .error_for_status()
            .context("control plane rejected runtime fetch")?
            .json::<RuntimeDescriptor>()
            .await
            .context("decode control-plane runtime descriptor")?;

        if descriptor.status == RuntimeStatus::Ready {
            return Ok(descriptor);
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!(
                "timed out waiting for runtime '{runtime_key}' to become ready during resume"
            ));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
