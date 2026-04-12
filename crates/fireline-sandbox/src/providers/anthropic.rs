use std::collections::{BTreeMap, HashMap};
use std::env;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use durable_streams::Client as DurableStreamsClient;
use futures::StreamExt;
use reqwest::{Client as HttpClient, Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::provider_model::{
    ExecutionResult, ProviderCapabilities, SandboxConfig, SandboxDescriptor, SandboxHandle,
    SandboxProvider, SandboxStatus,
};
use fireline_session::Endpoint;

const ANTHROPIC_PROVIDER_NAME: &str = "anthropic";
const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_BETA: &str = "managed-agents-2026-04-01";
const DEFAULT_MODEL: &str = "claude-sonnet-4-6";
const DEFAULT_API_BASE_URL: &str = "https://api.anthropic.com";

#[derive(Debug, Clone)]
pub struct RemoteAnthropicProviderConfig {
    pub api_base_url: String,
    pub anthropic_version: String,
    pub anthropic_beta: String,
    pub default_model: String,
    pub request_timeout: Duration,
}

impl Default for RemoteAnthropicProviderConfig {
    fn default() -> Self {
        Self {
            api_base_url: DEFAULT_API_BASE_URL.to_string(),
            anthropic_version: ANTHROPIC_VERSION.to_string(),
            anthropic_beta: ANTHROPIC_BETA.to_string(),
            default_model: DEFAULT_MODEL.to_string(),
            request_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Clone)]
pub struct RemoteAnthropicProvider {
    http: HttpClient,
    config: RemoteAnthropicProviderConfig,
    sandboxes: std::sync::Arc<Mutex<HashMap<String, AnthropicSandboxRecord>>>,
}

impl RemoteAnthropicProvider {
    pub fn new(config: RemoteAnthropicProviderConfig) -> Result<Self> {
        let http = HttpClient::builder()
            .timeout(config.request_timeout)
            .build()
            .context("build Anthropic managed-agents HTTP client")?;
        Ok(Self {
            http,
            config,
            sandboxes: std::sync::Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn api_key(&self) -> Result<String> {
        env::var(ANTHROPIC_API_KEY_ENV).with_context(|| {
            format!(
                "Anthropic provider requires {ANTHROPIC_API_KEY_ENV} to be set in the environment"
            )
        })
    }

    fn endpoint_headers(&self) -> Result<BTreeMap<String, String>> {
        let api_key = self.api_key()?;
        Ok(BTreeMap::from([
            ("x-api-key".to_string(), api_key),
            (
                "anthropic-version".to_string(),
                self.config.anthropic_version.clone(),
            ),
            (
                "anthropic-beta".to_string(),
                self.config.anthropic_beta.clone(),
            ),
            ("accept".to_string(), "text/event-stream".to_string()),
        ]))
    }

    fn request(&self, method: Method, path: &str) -> Result<reqwest::RequestBuilder> {
        let api_key = self.api_key()?;
        let url = format!("{}{}", self.config.api_base_url.trim_end_matches('/'), path);
        Ok(self
            .http
            .request(method, url)
            .header("x-api-key", api_key)
            .header("anthropic-version", &self.config.anthropic_version)
            .header("anthropic-beta", &self.config.anthropic_beta))
    }

    async fn request_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T> {
        let request = self.request(method, path)?;
        let response = if let Some(body) = body {
            request.json(&body)
        } else {
            request
        }
        .send()
        .await
        .with_context(|| format!("Anthropic request {path}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("Anthropic request {path} failed: {status} {body}"));
        }

        response
            .json::<T>()
            .await
            .with_context(|| format!("decode Anthropic response for {path}"))
    }

    async fn request_optional_json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
    ) -> Result<Option<T>> {
        let request = self.request(method, path)?;
        let response = request
            .send()
            .await
            .with_context(|| format!("Anthropic request {path}"))?;

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("Anthropic request {path} failed: {status} {body}"));
        }

        Ok(Some(
            response
                .json::<T>()
                .await
                .with_context(|| format!("decode Anthropic response for {path}"))?,
        ))
    }

    async fn request_empty(&self, method: Method, path: &str) -> Result<StatusCode> {
        let request = self.request(method, path)?;
        let response = request
            .send()
            .await
            .with_context(|| format!("Anthropic request {path}"))?;
        Ok(response.status())
    }

    async fn create_agent(&self, config: &SandboxConfig) -> Result<AnthropicAgentResource> {
        let model = agent_model(config, &self.config.default_model);
        let system = build_system_prompt(config);
        let permission_policy = permission_policy(config);
        let body = json!({
            "name": config.name,
            "model": model,
            "system": system,
            "tools": [
                {
                    "type": "agent_toolset_20260401",
                    "default_config": {
                        "permission_policy": { "type": permission_policy }
                    }
                }
            ],
            "metadata": config.labels,
        });
        self.request_json(Method::POST, "/v1/agents", Some(body)).await
    }

    async fn create_environment(
        &self,
        config: &SandboxConfig,
    ) -> Result<AnthropicEnvironmentResource> {
        if !config.resources.is_empty() {
            return Err(anyhow!(
                "Anthropic provider does not yet support Fireline resource mounts"
            ));
        }

        let body = json!({
            "name": format!("{}-environment", config.name),
            "config": environment_config(config),
        });
        self.request_json(Method::POST, "/v1/environments", Some(body))
            .await
    }

    async fn create_session(
        &self,
        agent: &AnthropicAgentResource,
        environment: &AnthropicEnvironmentResource,
    ) -> Result<AnthropicSessionResource> {
        let body = json!({
            "agent": {
                "type": "agent",
                "id": agent.id,
                "version": agent.version,
            },
            "environment_id": environment.id,
        });
        self.request_json(Method::POST, "/v1/sessions", Some(body)).await
    }

    async fn fetch_session(&self, id: &str) -> Result<Option<AnthropicSessionResource>> {
        self.request_optional_json(Method::GET, &format!("/v1/sessions/{id}"))
            .await
    }

    async fn list_sessions(&self) -> Result<Vec<AnthropicSessionResource>> {
        let response: AnthropicSessionListResponse =
            self.request_json(Method::GET, "/v1/sessions", None).await?;
        Ok(response.data)
    }

    async fn list_events(&self, id: &str) -> Result<Vec<Value>> {
        let response: AnthropicEventListResponse = self
            .request_json(Method::GET, &format!("/v1/sessions/{id}/events"), None)
            .await?;
        Ok(response.data)
    }

    async fn append_user_message(&self, id: &str, text: &str) -> Result<()> {
        let body = json!({
            "events": [
                {
                    "type": "user.message",
                    "content": [
                        {
                            "type": "text",
                            "text": text,
                        }
                    ]
                }
            ]
        });
        let _: Value = self
            .request_json(Method::POST, &format!("/v1/sessions/{id}/events"), Some(body))
            .await?;
        Ok(())
    }

    async fn archive_session(&self, id: &str) -> Result<bool> {
        let status = self
            .request_empty(Method::POST, &format!("/v1/sessions/{id}/archive"))
            .await?;
        Ok(status != StatusCode::NOT_FOUND)
    }

    fn acp_endpoint(&self, session_id: &str) -> Result<Endpoint> {
        Ok(Endpoint {
            url: format!(
                "{}/v1/sessions/{session_id}/stream?beta=true",
                self.config.api_base_url.trim_end_matches('/')
            ),
            headers: Some(self.endpoint_headers()?),
        })
    }

    fn state_stream_name(config: &SandboxConfig, session_id: &str) -> String {
        config
            .state_stream
            .clone()
            .unwrap_or_else(|| format!("anthropic-session-{}", sanitize_state_stream_key(session_id)))
    }

    fn state_endpoint(&self, config: &SandboxConfig, session_id: &str) -> Endpoint {
        Endpoint::new(join_stream_url(
            &config.durable_streams_url,
            &Self::state_stream_name(config, session_id),
        ))
    }

    fn direct_event_endpoint(&self, session_id: &str) -> Result<Endpoint> {
        Ok(Endpoint {
            url: format!(
                "{}/v1/sessions/{session_id}/events",
                self.config.api_base_url.trim_end_matches('/')
            ),
            headers: Some(self.endpoint_headers()?),
        })
    }

    fn descriptor_from_session(
        &self,
        session: &AnthropicSessionResource,
        fallback: Option<&SandboxDescriptor>,
    ) -> Result<SandboxDescriptor> {
        let cached = fallback;
        Ok(SandboxDescriptor {
            id: session.id.clone(),
            provider: ANTHROPIC_PROVIDER_NAME.to_string(),
            status: cached
                .map(|descriptor| descriptor.status.clone())
                .unwrap_or_else(|| map_session_status(session.status.as_deref())),
            acp: self.acp_endpoint(&session.id)?,
            state: cached
                .map(|descriptor| descriptor.state.clone())
                .unwrap_or(self.direct_event_endpoint(&session.id)?),
            labels: cached
                .map(|descriptor| descriptor.labels.clone())
                .unwrap_or_default(),
            created_at_ms: cached
                .map(|descriptor| descriptor.created_at_ms)
                .unwrap_or_else(now_ms),
            updated_at_ms: now_ms(),
        })
    }

    fn relay_session_stream(
        &self,
        session_id: String,
        state_stream_url: String,
    ) -> JoinHandle<()> {
        let provider = self.clone();
        tokio::spawn(async move {
            if let Err(error) = provider
                .relay_session_stream_inner(&session_id, &state_stream_url)
                .await
            {
                warn!(
                    session_id,
                    error = %error,
                    "Anthropic session relay stopped unexpectedly"
                );
            }
        })
    }

    async fn relay_session_stream_inner(
        &self,
        session_id: &str,
        state_stream_url: &str,
    ) -> Result<()> {
        let request = self
            .request(
                Method::GET,
                &format!("/v1/sessions/{session_id}/stream?beta=true"),
            )?
            .header("accept", "text/event-stream");
        let response = request
            .send()
            .await
            .with_context(|| format!("open Anthropic SSE stream for session {session_id}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Anthropic stream for session {session_id} failed: {status} {body}"
            ));
        }

        let client = DurableStreamsClient::new();
        let mut stream = client.stream(state_stream_url);
        stream.set_content_type("application/json");
        let producer = stream
            .producer(format!("anthropic-events-{session_id}"))
            .content_type("application/json")
            .build();

        let mut chunks = response.bytes_stream();
        let mut buffer = String::new();
        let mut data_lines = Vec::new();
        let mut event_index: u64 = 0;

        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.with_context(|| {
                format!("read Anthropic SSE chunk for session {session_id}")
            })?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline) = buffer.find('\n') {
                let line = buffer.drain(..=newline).collect::<String>();
                let line = line.trim_end_matches(['\r', '\n']);
                if line.is_empty() {
                    if !data_lines.is_empty() {
                        let payload = data_lines.join("\n");
                        data_lines.clear();
                        if payload != "[DONE]" {
                            let event: Value = serde_json::from_str(&payload).with_context(|| {
                                format!("decode Anthropic SSE event for session {session_id}")
                            })?;
                            producer.append_json(&AnthropicRelayEnvelope {
                                entity_type: "anthropic_session_event",
                                key: format!("{session_id}:{event_index}"),
                                headers: RelayHeaders { operation: "append" },
                                value: AnthropicRelayEvent {
                                    session_id: session_id.to_string(),
                                    provider: ANTHROPIC_PROVIDER_NAME,
                                    event,
                                },
                            });
                            producer
                                .flush()
                                .await
                                .with_context(|| {
                                    format!(
                                        "append relayed Anthropic event {event_index} for session {session_id}"
                                    )
                                })?;
                            event_index += 1;
                        }
                    }
                    continue;
                }

                if let Some(data) = line.strip_prefix("data:") {
                    data_lines.push(data.trim_start().to_string());
                }
            }
        }

        Ok(())
    }
}

#[async_trait]
impl SandboxProvider for RemoteAnthropicProvider {
    fn name(&self) -> &str {
        ANTHROPIC_PROVIDER_NAME
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            file_transfer: true,
            ..ProviderCapabilities::default()
        }
    }

    async fn create(&self, config: &SandboxConfig) -> Result<SandboxHandle> {
        let agent = self.create_agent(config).await?;
        let environment = self.create_environment(config).await?;
        let session = self.create_session(&agent, &environment).await?;

        let descriptor = SandboxDescriptor {
            id: session.id.clone(),
            provider: ANTHROPIC_PROVIDER_NAME.to_string(),
            status: map_session_status(session.status.as_deref()),
            acp: self.acp_endpoint(&session.id)?,
            state: self.state_endpoint(config, &session.id),
            labels: config.labels.clone(),
            created_at_ms: now_ms(),
            updated_at_ms: now_ms(),
        };
        let relay_task = self.relay_session_stream(
            session.id.clone(),
            descriptor.state.url.clone(),
        );

        self.sandboxes.lock().await.insert(
            session.id.clone(),
            AnthropicSandboxRecord {
                descriptor: descriptor.clone(),
                relay_task: Some(relay_task),
            },
        );

        info!(
            session_id = descriptor.id,
            state_stream = descriptor.state.url,
            "created Anthropic managed-agent sandbox"
        );

        Ok(SandboxHandle::from_descriptor(descriptor, self.name()))
    }

    async fn get(&self, id: &str) -> Result<Option<SandboxDescriptor>> {
        let cached = {
            let guard = self.sandboxes.lock().await;
            guard.get(id).map(|record| record.descriptor.clone())
        };
        if cached
            .as_ref()
            .is_some_and(|descriptor| descriptor.status == SandboxStatus::Stopped)
        {
            return Ok(cached);
        }

        let Some(session) = self.fetch_session(id).await? else {
            return Ok(cached);
        };

        let descriptor = self.descriptor_from_session(&session, cached.as_ref())?;
        if let Some(record) = self.sandboxes.lock().await.get_mut(id) {
            record.descriptor = descriptor.clone();
        }
        Ok(Some(descriptor))
    }

    async fn list(
        &self,
        labels: Option<&HashMap<String, String>>,
    ) -> Result<Vec<SandboxDescriptor>> {
        let cached = {
            let guard = self.sandboxes.lock().await;
            guard
                .iter()
                .map(|(id, record)| (id.clone(), record.descriptor.clone()))
                .collect::<HashMap<_, _>>()
        };
        let mut descriptors = Vec::new();

        if self.api_key().is_ok() {
            for session in self.list_sessions().await? {
                let descriptor =
                    self.descriptor_from_session(&session, cached.get(&session.id))?;
                if labels_match(&descriptor.labels, labels) {
                    descriptors.push(descriptor);
                }
            }
        }

        for (id, descriptor) in cached {
            if descriptors.iter().any(|descriptor| descriptor.id == id) {
                continue;
            }
            if labels_match(&descriptor.labels, labels) {
                descriptors.push(descriptor);
            }
        }

        descriptors.sort_by(|left, right| left.id.cmp(&right.id));
        descriptors.dedup_by(|left, right| left.id == right.id);
        Ok(descriptors)
    }

    async fn execute(
        &self,
        id: &str,
        command: &str,
        timeout: Option<Duration>,
        _env: Option<&HashMap<String, String>>,
    ) -> Result<ExecutionResult> {
        let started = Instant::now();
        let initial_count = self.list_events(id).await?.len();
        let prompt = format!(
            "Run the following shell command exactly once using the available built-in tools. \
Return the command output directly. Command:\n```sh\n{command}\n```"
        );
        self.append_user_message(id, &prompt).await?;

        let deadline = Instant::now() + timeout.unwrap_or(self.config.request_timeout);
        loop {
            let Some(session) = self.fetch_session(id).await? else {
                return Err(anyhow!("Anthropic session '{id}' not found"));
            };
            match session.status.as_deref() {
                Some("idle") => break,
                Some("terminated") => {
                    let events = self.list_events(id).await?;
                    let stderr = extract_session_errors(events.iter().skip(initial_count));
                    return Ok(ExecutionResult {
                        exit_code: 1,
                        stdout: String::new(),
                        stderr,
                        duration_ms: started.elapsed().as_millis() as u64,
                        timed_out: false,
                    });
                }
                _ => {}
            }

            if Instant::now() >= deadline {
                return Ok(ExecutionResult {
                    exit_code: 124,
                    stdout: String::new(),
                    stderr: format!("timed out waiting for Anthropic session '{id}' to go idle"),
                    duration_ms: started.elapsed().as_millis() as u64,
                    timed_out: true,
                });
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }

        let events = self.list_events(id).await?;
        let new_events = events.iter().skip(initial_count);
        let stderr = extract_session_errors(new_events.clone());
        let stdout = extract_agent_message_text(new_events);
        Ok(ExecutionResult {
            exit_code: if stderr.is_empty() { 0 } else { 1 },
            stdout,
            stderr,
            duration_ms: started.elapsed().as_millis() as u64,
            timed_out: false,
        })
    }

    async fn destroy(&self, id: &str) -> Result<bool> {
        let archived = self.archive_session(id).await?;
        if !archived {
            return Ok(false);
        }

        if let Some(record) = self.sandboxes.lock().await.get_mut(id) {
            if let Some(task) = record.relay_task.take() {
                task.abort();
            }
            record.descriptor.status = SandboxStatus::Stopped;
            record.descriptor.updated_at_ms = now_ms();
        }

        Ok(true)
    }

    async fn health_check(&self) -> Result<bool> {
        if self.api_key().is_err() {
            return Ok(false);
        }

        let request = self.request(Method::GET, "/v1/sessions")?;
        let response = request.send().await;
        Ok(response.is_ok_and(|response| response.status().is_success()))
    }
}

#[derive(Debug)]
struct AnthropicSandboxRecord {
    descriptor: SandboxDescriptor,
    relay_task: Option<JoinHandle<()>>,
}

#[derive(Debug, Deserialize)]
struct AnthropicAgentResource {
    id: String,
    version: Value,
}

#[derive(Debug, Deserialize)]
struct AnthropicEnvironmentResource {
    id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicSessionResource {
    id: String,
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicSessionListResponse {
    #[serde(default)]
    data: Vec<AnthropicSessionResource>,
}

#[derive(Debug, Deserialize)]
struct AnthropicEventListResponse {
    #[serde(default)]
    data: Vec<Value>,
}

#[derive(Debug, Serialize)]
struct RelayHeaders {
    operation: &'static str,
}

#[derive(Debug, Serialize)]
struct AnthropicRelayEnvelope {
    #[serde(rename = "type")]
    entity_type: &'static str,
    key: String,
    headers: RelayHeaders,
    value: AnthropicRelayEvent,
}

#[derive(Debug, Serialize)]
struct AnthropicRelayEvent {
    session_id: String,
    provider: &'static str,
    event: Value,
}

fn agent_model(config: &SandboxConfig, default_model: &str) -> String {
    config
        .agent_command
        .first()
        .cloned()
        .unwrap_or_else(|| default_model.to_string())
}

fn build_system_prompt(config: &SandboxConfig) -> Option<String> {
    let mut sections = Vec::new();

    if let Some(system) = config.env_vars.get("FIRELINE_ANTHROPIC_SYSTEM") {
        sections.push(system.clone());
    }

    if config.agent_command.len() > 1 {
        sections.push(config.agent_command[1..].join(" "));
    }

    for component in &config.topology.components {
        if component.name != "context_injection" {
            continue;
        }

        let Some(config) = component.config.as_ref() else {
            continue;
        };
        let Ok(context) = serde_json::from_value::<ContextInjectionConfig>(config.clone()) else {
            continue;
        };
        if let Some(prepend_text) = context.prepend_text {
            sections.push(prepend_text);
        }
        for source in context.sources {
            match source {
                ContextSourceSpec::StaticText { text } => sections.push(text),
                ContextSourceSpec::WorkspaceFile { path } => {
                    sections.push(format!("Relevant workspace file path: {path}"));
                }
                ContextSourceSpec::Datetime => {}
            }
        }
    }

    if sections.is_empty() {
        None
    } else {
        Some(sections.join("\n\n"))
    }
}

fn permission_policy(config: &SandboxConfig) -> &'static str {
    if config
        .topology
        .components
        .iter()
        .any(|component| component.name == "approval_gate")
    {
        "always_ask"
    } else {
        "always_allow"
    }
}

fn environment_config(config: &SandboxConfig) -> Value {
    let networking_type = config
        .env_vars
        .get("FIRELINE_ANTHROPIC_NETWORKING_TYPE")
        .map(String::as_str)
        .unwrap_or("unrestricted");
    match networking_type {
        "limited" => {
            let allowed_hosts = config
                .env_vars
                .get("FIRELINE_ANTHROPIC_ALLOWED_HOSTS")
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|entry| !entry.is_empty())
                        .map(ToOwned::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            json!({
                "type": "cloud",
                "networking": {
                    "type": "limited",
                    "allowed_hosts": allowed_hosts,
                    "allow_mcp_servers": bool_env(config, "FIRELINE_ANTHROPIC_ALLOW_MCP_SERVERS"),
                    "allow_package_managers": bool_env(config, "FIRELINE_ANTHROPIC_ALLOW_PACKAGE_MANAGERS"),
                }
            })
        }
        _ => json!({
            "type": "cloud",
            "networking": { "type": "unrestricted" },
        }),
    }
}

fn bool_env(config: &SandboxConfig, key: &str) -> bool {
    config
        .env_vars
        .get(key)
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
}

fn labels_match(
    actual: &HashMap<String, String>,
    expected: Option<&HashMap<String, String>>,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };

    expected
        .iter()
        .all(|(key, value)| actual.get(key).is_some_and(|actual_value| actual_value == value))
}

fn map_session_status(status: Option<&str>) -> SandboxStatus {
    match status {
        Some("idle") => SandboxStatus::Idle,
        Some("running") => SandboxStatus::Busy,
        Some("rescheduling") => SandboxStatus::Creating,
        Some("terminated") => SandboxStatus::Broken,
        _ => SandboxStatus::Creating,
    }
}

fn join_stream_url(base: &str, stream_name: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), stream_name)
}

fn sanitize_state_stream_key(value: &str) -> String {
    value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => ch,
            _ => '-',
        })
        .collect()
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time went backwards")
        .as_millis() as i64
}

fn extract_agent_message_text<'a>(events: impl Iterator<Item = &'a Value>) -> String {
    events
        .filter_map(|event| {
            if event.get("type").and_then(Value::as_str) != Some("agent.message") {
                return None;
            }
            let text_blocks = event
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|block| {
                    (block.get("type").and_then(Value::as_str) == Some("text"))
                        .then(|| block.get("text").and_then(Value::as_str))
                        .flatten()
                        .map(ToOwned::to_owned)
                })
                .collect::<Vec<_>>();
            (!text_blocks.is_empty()).then(|| text_blocks.join("\n"))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn extract_session_errors<'a>(events: impl Iterator<Item = &'a Value>) -> String {
    events
        .filter_map(|event| {
            (event.get("type").and_then(Value::as_str) == Some("session.error"))
                .then(|| {
                    event.get("error")
                        .and_then(|error| error.get("message"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextInjectionConfig {
    #[serde(default)]
    prepend_text: Option<String>,
    #[serde(default)]
    sources: Vec<ContextSourceSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ContextSourceSpec {
    Datetime,
    WorkspaceFile { path: String },
    StaticText { text: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use fireline_session::{TopologyComponentSpec, TopologySpec};

    #[test]
    fn approval_gate_maps_to_always_ask() {
        let config = SandboxConfig {
            name: "test".to_string(),
            agent_command: vec!["claude-sonnet-4-6".to_string()],
            topology: TopologySpec {
                components: vec![TopologyComponentSpec {
                    name: "approval_gate".to_string(),
                    config: None,
                }],
            },
            resources: Vec::new(),
            durable_streams_url: "http://localhost:8787/v1/stream".to_string(),
            state_stream: None,
            env_vars: HashMap::new(),
            labels: HashMap::new(),
            provider: Some("anthropic".to_string()),
        };

        assert_eq!(permission_policy(&config), "always_ask");
    }

    #[test]
    fn limited_networking_uses_allowed_hosts_env() {
        let config = SandboxConfig {
            name: "test".to_string(),
            agent_command: vec!["claude-sonnet-4-6".to_string()],
            topology: TopologySpec::default(),
            resources: Vec::new(),
            durable_streams_url: "http://localhost:8787/v1/stream".to_string(),
            state_stream: None,
            env_vars: HashMap::from([
                (
                    "FIRELINE_ANTHROPIC_NETWORKING_TYPE".to_string(),
                    "limited".to_string(),
                ),
                (
                    "FIRELINE_ANTHROPIC_ALLOWED_HOSTS".to_string(),
                    "https://api.example.com,https://mcp.example.com".to_string(),
                ),
            ]),
            labels: HashMap::new(),
            provider: Some("anthropic".to_string()),
        };

        let networking = environment_config(&config);
        assert_eq!(
            networking["networking"]["allowed_hosts"],
            json!(["https://api.example.com", "https://mcp.example.com"])
        );
    }
}
