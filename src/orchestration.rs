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
    let runtime = lookup_runtime_for_session(http, control_plane_url, session_id).await?;

    if runtime.status == RuntimeStatus::Ready {
        return Ok(runtime);
    }

    let persisted = lookup_runtime_spec_for_session(http, control_plane_url, session_id).await?;
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
    let index = materialize_session_index(state_stream_url).await?;
    index
        .runtime_spec(runtime_key)
        .await
        .ok_or_else(|| anyhow!("runtime_spec '{runtime_key}' not found in state stream"))
}

pub async fn materialize_session_index(state_stream_url: &str) -> Result<SessionIndex> {
    let index = SessionIndex::new();
    let materializer = RuntimeMaterializer::new(vec![Arc::new(index.clone())]);
    let task = materializer.connect(state_stream_url.to_string());
    task.preload().await?;
    task.abort();
    Ok(index)
}

async fn lookup_runtime_for_session(
    http: &HttpClient,
    control_plane_url: &str,
    session_id: &str,
) -> Result<RuntimeDescriptor> {
    let runtimes = list_runtimes(http, control_plane_url).await?;
    for runtime in runtimes {
        let index = materialize_session_index(&runtime.state.url).await?;
        if index.get(session_id).await.is_some() {
            return Ok(runtime);
        }
    }

    Err(anyhow!("session '{session_id}' not found in control-plane runtimes"))
}

async fn lookup_runtime_spec_for_session(
    http: &HttpClient,
    control_plane_url: &str,
    session_id: &str,
) -> Result<PersistedRuntimeSpec> {
    let runtimes = list_runtimes(http, control_plane_url).await?;
    for runtime in runtimes {
        let index = materialize_session_index(&runtime.state.url).await?;
        if let Some(spec) = index.runtime_spec_for_session(session_id).await {
            return Ok(spec);
        }
    }

    Err(anyhow!(
        "runtime_spec for session '{session_id}' not found in control-plane runtimes"
    ))
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
