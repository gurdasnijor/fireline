use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use durable_streams::{Client as DurableStreamsClient, CreateOptions, DurableStream, Producer};
use fireline_components::audit::{AuditConfig, AuditSink, AuditTracer};
use fireline_components::context::{
    ContextConfig, ContextInjectionComponent, ContextPlacement, ContextSource, DatetimeSource,
    WorkspaceFileSource,
};
use fireline_components::lookup::{ActiveTurnLookup, ChildSessionEdgeSink};
use fireline_components::PeerComponent;
use fireline_conductor::topology::{TopologyRegistry, TopologySpec, TraceWriterInstance};
use serde::Deserialize;

#[derive(Clone)]
#[allow(dead_code)]
pub struct ComponentContext {
    pub runtime_key: String,
    pub runtime_id: String,
    pub node_id: String,
    pub stream_base_url: String,
    pub peer_directory_path: PathBuf,
    pub active_turn_lookup: Arc<dyn ActiveTurnLookup>,
    pub child_session_edge_sink: Arc<dyn ChildSessionEdgeSink>,
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

pub async fn ensure_named_streams(
    stream_base_url: &str,
    stream_names: &[String],
) -> Result<()> {
    let client = DurableStreamsClient::new();
    for stream_name in stream_names {
        let url = stream_url(stream_base_url, stream_name);
        let mut stream = client.stream(&url);
        stream.set_content_type("application/json");
        ensure_stream_exists(&stream).await?;
    }
    Ok(())
}

pub fn build_runtime_topology_registry(context: ComponentContext) -> TopologyRegistry {
    TopologyRegistry::builder()
        .register_component("peer_mcp", {
            let context = context.clone();
            move |_config| {
            Ok(sacp::DynConnectTo::new(PeerComponent::new(
                context.peer_directory_path.clone(),
                context.active_turn_lookup.clone(),
                context.child_session_edge_sink.clone(),
                context.runtime_id.clone(),
            )))
        }})
        .register_tracer("audit", {
            let context = context.clone();
            move |config| {
            let config = config
                .ok_or_else(|| anyhow!("topology component 'audit' requires config"))?;
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
        }})
        .register_component("context_injection", move |config| {
            let config = config.ok_or_else(|| {
                anyhow!("topology component 'context_injection' requires config")
            })?;
            let parsed: ContextInjectionConfig =
                serde_json::from_value(config.clone()).context("parse context injection config")?;
            Ok(sacp::DynConnectTo::new(ContextInjectionComponent::new(
                build_context_config(parsed),
            )))
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
