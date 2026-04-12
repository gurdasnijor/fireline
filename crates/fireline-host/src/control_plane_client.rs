use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use fireline_session::{HeartbeatMetrics, HeartbeatReport, RuntimeRegistration};
use reqwest::{Client as HttpClient, StatusCode};
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct ControlPlaneClient {
    http: HttpClient,
    base_url: String,
    token: String,
    runtime_key: String,
}

impl ControlPlaneClient {
    pub fn new(
        base_url: impl Into<String>,
        token: impl Into<String>,
        runtime_key: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            http: HttpClient::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .context("build control-plane http client")?,
            base_url: base_url.into(),
            token: token.into(),
            runtime_key: runtime_key.into(),
        })
    }

    pub async fn register(&self, registration: RuntimeRegistration) -> Result<()> {
        let url = format!(
            "{}/v1/runtimes/{}/register",
            self.base_url.trim_end_matches('/'),
            self.runtime_key
        );
        let mut backoff = Duration::from_millis(250);
        for attempt in 0..3 {
            let result = self
                .http
                .post(&url)
                .bearer_auth(&self.token)
                .json(&registration)
                .send()
                .await;

            match result {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) if response.status() == StatusCode::CONFLICT => {
                    return Err(anyhow!(
                        "control plane rejected registration for runtime '{}' with 409",
                        self.runtime_key
                    ));
                }
                Ok(response) => {
                    tracing::warn!(
                        attempt,
                        status = %response.status(),
                        runtime_key = %self.runtime_key,
                        "control-plane registration attempt failed"
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        attempt,
                        ?error,
                        runtime_key = %self.runtime_key,
                        "control-plane registration transport error"
                    );
                }
            }

            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(2));
        }

        Err(anyhow!(
            "control plane registration failed for runtime '{}' after 3 attempts",
            self.runtime_key
        ))
    }

    pub fn spawn_heartbeat_loop(
        self: &Arc<Self>,
        metrics_source: impl Fn() -> HeartbeatMetrics + Send + Sync + 'static,
    ) -> JoinHandle<()> {
        let client = self.clone();
        tokio::spawn(async move {
            let url = format!(
                "{}/v1/runtimes/{}/heartbeat",
                client.base_url.trim_end_matches('/'),
                client.runtime_key
            );
            loop {
                let report = HeartbeatReport {
                    ts_ms: now_ms(),
                    metrics: Some(metrics_source()),
                };
                let result = client
                    .http
                    .post(&url)
                    .bearer_auth(&client.token)
                    .json(&report)
                    .send()
                    .await;
                if let Err(error) = result.and_then(|response| response.error_for_status()) {
                    tracing::warn!(
                        ?error,
                        runtime_key = %client.runtime_key,
                        "control-plane heartbeat failed"
                    );
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}
