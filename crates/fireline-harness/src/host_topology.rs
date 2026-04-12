use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, DurableStream, Producer};
use fireline_resources::{
    FileBackend, FsBackendComponent, FsBackendConfig, LocalFileBackend, MountedResource,
    StreamFsFileBackend,
};
use fireline_tools::attach::AttachToolComponent;
use fireline_tools::directory::PeerRegistry;
use fireline_tools::lookup::ActiveTurnLookup;
use fireline_tools::peer;
use fireline_tools::{CapabilityRef, PeerComponent, emit_tool_descriptors};
use serde::Deserialize;

use crate::approval::{
    ApprovalAction, ApprovalConfig, ApprovalGateComponent, ApprovalMatch, ApprovalPolicy,
};
use crate::audit::{AuditConfig, AuditSink, AuditTracer};
use crate::budget::{BudgetAction, BudgetComponent, BudgetConfig};
use crate::context::{
    ContextConfig, ContextInjectionComponent, ContextPlacement, ContextSource, DatetimeSource,
    WorkspaceFileSource,
};
use crate::secrets::{
    InjectionRule, InjectionScope, InjectionTarget, LocalCredentialResolver,
    SecretsInjectionComponent,
};
use crate::topology::{TopologyRegistry, TopologySpec, TraceWriterInstance};

#[derive(Clone)]
#[allow(dead_code)]
pub struct ComponentContext {
    pub host_key: String,
    pub host_id: String,
    pub node_id: String,
    pub stream_base_url: String,
    pub state_stream_url: String,
    pub state_producer: Producer,
    pub peer_registry: Arc<dyn PeerRegistry>,
    pub active_turn_lookup: Arc<dyn ActiveTurnLookup>,
    pub mounted_resources: Vec<MountedResource>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditComponentConfig {
    pub stream_name: String,
    #[serde(default)]
    pub include_methods: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextInjectionConfig {
    #[serde(default)]
    pub prepend_text: Option<String>,
    #[serde(default)]
    pub placement: ContextPlacementConfig,
    #[serde(default)]
    pub sources: Vec<ContextSourceSpec>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetComponentConfig {
    #[serde(default)]
    pub max_tokens: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalGateComponentConfig {
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub policies: Vec<ApprovalPolicyConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsInjectionConfig {
    #[serde(default)]
    pub bindings: Vec<SecretsBindingConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsBindingConfig {
    pub name: String,
    #[serde(rename = "ref")]
    pub credential_ref: String,
    #[allow(dead_code)]
    #[serde(default, rename = "allow")]
    pub allowed_domains: Vec<String>,
}

/// Config for the `attach_tool` topology component (slice 17
/// Capability profiles). Each entry in `capabilities` is a portable
/// attachment that projects to a single `tool_descriptor` state
/// envelope on conductor wire-up; see
/// [`fireline_tools::attach`] for the first-attach-wins
/// collision rule and the slice 17 scope boundary.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachToolConfig {
    #[serde(default)]
    pub capabilities: Vec<CapabilityRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalPolicyConfig {
    pub r#match: ApprovalMatchConfig,
    pub action: ApprovalActionConfig,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ApprovalMatchConfig {
    PromptContains { needle: String },
    Tool { name: String },
    ToolPrefix { prefix: String },
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalActionConfig {
    RequireApproval,
    Deny,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPlacementConfig {
    #[default]
    Prepend,
    Append,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ContextSourceSpec {
    Datetime,
    WorkspaceFile { path: PathBuf },
    StaticText { text: String },
}

pub fn audit_stream_names(topology: &TopologySpec) -> Result<Vec<String>> {
    topology
        .components
        .iter()
        .filter(|component| component.name == "audit")
        .map(|component| {
            let config = component
                .config
                .as_ref()
                .ok_or_else(|| anyhow!("topology component 'audit' requires config"))?;
            let parsed: AuditComponentConfig =
                serde_json::from_value(config.clone()).context("parse audit config")?;
            Ok(parsed.stream_name)
        })
        .collect()
}

pub async fn ensure_named_streams(stream_base_url: &str, stream_names: &[String]) -> Result<()> {
    let client = DurableStreamsClient::new();
    for stream_name in stream_names {
        let url = stream_url(stream_base_url, stream_name);
        let mut stream = client.stream(&url);
        stream.set_content_type("application/json");
        ensure_stream_exists(&stream).await?;
    }
    Ok(())
}

pub fn build_host_topology_registry(context: ComponentContext) -> TopologyRegistry {
    TopologyRegistry::builder()
        .register_component("peer_mcp", {
            let context = context.clone();
            move |_config| {
                // Mirror the peer MCP server's registered tool surface
                // onto the durable state stream as `tool_descriptor`
                // envelopes. The schema-only `{name, description,
                // input_schema}` triple is the contract Anthropic's
                // Tools primitive specifies; emitting it at component-
                // registration time (once per conductor build) gives
                // tests and external subscribers a typed view of the
                // tool surface without having to reach through the MCP
                // wire. Inserts with the same `{source}:{tool_name}`
                // key project to the same record, so repeated conductor
                // builds are idempotent on the projection side.
                let producer = context.state_producer.clone();
                tokio::spawn(async move {
                    let descriptors = peer::tool_descriptors();
                    if let Err(error) =
                        emit_tool_descriptors(&producer, "peer_mcp", &descriptors).await
                    {
                        tracing::warn!(
                            %error,
                            "failed to emit peer_mcp tool_descriptor envelopes"
                        );
                    }
                });
                Ok(sacp::DynConnectTo::new(PeerComponent::new(
                    context.peer_registry.clone(),
                    context.active_turn_lookup.clone(),
                )))
            }
        })
        .register_tracer("audit", {
            let context = context.clone();
            move |config| {
                let config =
                    config.ok_or_else(|| anyhow!("topology component 'audit' requires config"))?;
                let parsed: AuditComponentConfig =
                    serde_json::from_value(config.clone()).context("parse audit config")?;
                let producer = build_named_producer(
                    &context.stream_base_url,
                    &parsed.stream_name,
                    format!("audit-{}", uuid::Uuid::new_v4()),
                );
                Ok(Box::new(AuditTracer::new(AuditConfig {
                    sink: AuditSink::DurableStream { producer },
                    include_methods: parsed.include_methods,
                })) as TraceWriterInstance)
            }
        })
        .register_component("context_injection", move |config| {
            let config = config
                .ok_or_else(|| anyhow!("topology component 'context_injection' requires config"))?;
            let parsed: ContextInjectionConfig =
                serde_json::from_value(config.clone()).context("parse context injection config")?;
            Ok(sacp::DynConnectTo::new(ContextInjectionComponent::new(
                build_context_config(parsed),
            )))
        })
        .register_component("budget", move |config| {
            let parsed = config
                .cloned()
                .map(serde_json::from_value::<BudgetComponentConfig>)
                .transpose()
                .context("parse budget config")?
                .unwrap_or(BudgetComponentConfig { max_tokens: None });
            Ok(sacp::DynConnectTo::new(BudgetComponent::new(
                BudgetConfig {
                    max_tokens: parsed.max_tokens,
                    max_tool_calls: None,
                    max_duration: None,
                    on_exceeded: BudgetAction::TerminateTurn,
                },
            )))
        })
        .register_component("approval_gate", {
            let context = context.clone();
            move |config| {
                let parsed = config
                    .cloned()
                    .map(serde_json::from_value::<ApprovalGateComponentConfig>)
                    .transpose()
                    .context("parse approval gate config")?
                    .unwrap_or(ApprovalGateComponentConfig {
                        timeout_ms: None,
                        policies: Vec::new(),
                    });
                let policies = parsed
                    .policies
                    .into_iter()
                    .map(|policy| ApprovalPolicy {
                        match_rule: match policy.r#match {
                            ApprovalMatchConfig::PromptContains { needle } => {
                                ApprovalMatch::PromptContains { needle }
                            }
                            ApprovalMatchConfig::Tool { name } => ApprovalMatch::Tool { name },
                            ApprovalMatchConfig::ToolPrefix { prefix } => {
                                ApprovalMatch::ToolPrefix { prefix }
                            }
                        },
                        action: match policy.action {
                            ApprovalActionConfig::RequireApproval => {
                                ApprovalAction::RequireApproval
                            }
                            ApprovalActionConfig::Deny => ApprovalAction::Deny,
                        },
                        reason: policy.reason,
                    })
                    .collect();
                let timeout = parsed.timeout_ms.map(std::time::Duration::from_millis);
                Ok(sacp::DynConnectTo::new(
                    ApprovalGateComponent::with_stream_and_timeout(
                        ApprovalConfig { policies },
                        Some(context.state_stream_url.clone()),
                        Some(context.state_producer.clone()),
                        timeout,
                    ),
                ))
            }
        })
        .register_component("secrets_injection", move |config| {
            let parsed = config
                .cloned()
                .map(serde_json::from_value::<SecretsInjectionConfig>)
                .transpose()
                .context("parse secrets injection config")?
                .unwrap_or(SecretsInjectionConfig {
                    bindings: Vec::new(),
                });
            let rules = parsed
                .bindings
                .into_iter()
                .map(|binding| {
                    Ok(InjectionRule {
                        target: InjectionTarget::EnvVar(binding.name),
                        credential_ref: parse_credential_ref(&binding.credential_ref)?,
                        scope: InjectionScope::Session,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(sacp::DynConnectTo::new(SecretsInjectionComponent::new(
                Arc::new(LocalCredentialResolver::default()),
                rules,
            )))
        })
        .register_tracer("durable_trace", |_config| {
            Ok(Box::new(NoopTraceWriter) as TraceWriterInstance)
        })
        .register_component("fs_backend", {
            let context = context.clone();
            move |config| {
                let config = config
                    .ok_or_else(|| anyhow!("topology component 'fs_backend' requires config"))?;
                let parsed: FsBackendConfig =
                    serde_json::from_value(config.clone()).context("parse fs backend config")?;
                let backend: Arc<dyn FileBackend> = match parsed {
                    FsBackendConfig::Local => {
                        Arc::new(LocalFileBackend::new(context.mounted_resources.clone()))
                    }
                    FsBackendConfig::StreamFs => Arc::new(StreamFsFileBackend::new(
                        context.state_stream_url.clone(),
                    )),
                };
                Ok(sacp::DynConnectTo::new(FsBackendComponent::new(
                    backend,
                    context.state_producer.clone(),
                )))
            }
        })
        .register_component("attach_tool", {
            let context = context.clone();
            move |config| {
                // Slice 17 capability-profiles factory. The config is
                // optional; an empty `capabilities` list is valid and
                // yields a no-op pass-through proxy. Each capability is
                // a portable `{descriptor, transport_ref, credential_ref}`
                // triple; the component emits only the descriptor half
                // onto the durable state stream, so the wire value stays
                // the Anthropic triple regardless of which transport the
                // capability is carrying at launch.
                let parsed = config
                    .cloned()
                    .map(serde_json::from_value::<AttachToolConfig>)
                    .transpose()
                    .context("parse attach_tool config")?
                    .unwrap_or(AttachToolConfig {
                        capabilities: Vec::new(),
                    });
                Ok(sacp::DynConnectTo::new(AttachToolComponent::new(
                    parsed.capabilities,
                    context.state_producer.clone(),
                )))
            }
        })
        .build()
}

fn build_context_config(config: ContextInjectionConfig) -> ContextConfig {
    let mut sources: Vec<Arc<dyn ContextSource>> = Vec::new();

    if let Some(prepend_text) = config.prepend_text.filter(|text| !text.is_empty()) {
        sources.push(Arc::new(StaticTextSource::new(prepend_text)));
    }

    for source in config.sources {
        sources.push(match source {
            ContextSourceSpec::Datetime => Arc::new(DatetimeSource),
            ContextSourceSpec::WorkspaceFile { path } => Arc::new(WorkspaceFileSource::new(path)),
            ContextSourceSpec::StaticText { text } => Arc::new(StaticTextSource::new(text)),
        });
    }

    ContextConfig {
        sources,
        placement: match config.placement {
            ContextPlacementConfig::Prepend => ContextPlacement::Prepend,
            ContextPlacementConfig::Append => ContextPlacement::Append,
        },
    }
}

fn parse_credential_ref(raw: &str) -> Result<fireline_tools::CredentialRef> {
    if let Some(var) = raw.strip_prefix("env:") {
        if var.is_empty() {
            return Err(anyhow!("invalid env credential ref '{raw}': missing variable name"));
        }
        return Ok(fireline_tools::CredentialRef::Env {
            var: var.to_string(),
        });
    }

    if let Some(key) = raw.strip_prefix("secret:") {
        if key.is_empty() {
            return Err(anyhow!("invalid secret credential ref '{raw}': missing key"));
        }
        return Ok(fireline_tools::CredentialRef::Secret {
            key: key.to_string(),
        });
    }

    if let Some(rest) = raw.strip_prefix("oauth:") {
        let mut parts = rest.split(':');
        let provider = parts.next().unwrap_or_default();
        let account = parts.next().map(str::to_string);
        if provider.is_empty() {
            return Err(anyhow!(
                "invalid oauth credential ref '{raw}': missing provider"
            ));
        }
        if parts.next().is_some() {
            return Err(anyhow!(
                "invalid oauth credential ref '{raw}': expected oauth:<provider>[:account]"
            ));
        }
        return Ok(fireline_tools::CredentialRef::OauthToken {
            provider: provider.to_string(),
            account,
        });
    }

    Err(anyhow!(
        "unsupported credential ref '{raw}': expected env:, secret:, or oauth:"
    ))
}

fn build_named_producer(stream_base_url: &str, stream_name: &str, producer_id: String) -> Producer {
    let client = DurableStreamsClient::new();
    let mut stream = client.stream(&stream_url(stream_base_url, stream_name));
    stream.set_content_type("application/json");
    stream
        .producer(producer_id)
        .content_type("application/json")
        .build()
}

fn stream_url(stream_base_url: &str, stream_name: &str) -> String {
    format!("{stream_base_url}/{stream_name}")
}

async fn ensure_stream_exists(stream: &DurableStream) -> Result<()> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match stream
            .create_with(CreateOptions::new().content_type("application/json"))
            .await
        {
            Ok(_) => return Ok(()),
            Err(err) => {
                if tokio::time::Instant::now() >= deadline {
                    return Err(anyhow::Error::from(err)).context("create named topology stream");
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
}

struct StaticTextSource {
    text: String,
}

impl StaticTextSource {
    fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

#[async_trait::async_trait]
impl ContextSource for StaticTextSource {
    async fn gather(&self, _session_id: &str) -> Result<String, sacp::Error> {
        Ok(self.text.clone())
    }
}

struct NoopTraceWriter;

impl sacp_conductor::trace::WriteEvent for NoopTraceWriter {
    fn write_event(&mut self, _event: &sacp_conductor::trace::TraceEvent) -> std::io::Result<()> {
        Ok(())
    }
}
