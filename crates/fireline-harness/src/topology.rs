use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
pub use fireline_session::{TopologyComponentSpec, TopologySpec};
use sacp::{Conductor, DynConnectTo};
use sacp_conductor::trace::WriteEvent;
use serde_json::Value;

pub use crate::host_topology::{
    ComponentContext, audit_stream_names, build_host_topology_registry, ensure_named_streams,
};

pub type ProxyComponentInstance = DynConnectTo<Conductor>;
pub type TraceWriterInstance = Box<dyn WriteEvent + Send>;

type ProxyFactory = dyn Fn(Option<&Value>) -> Result<ProxyComponentInstance> + Send + Sync;
type TraceFactory = dyn Fn(Option<&Value>) -> Result<TraceWriterInstance> + Send + Sync;

#[derive(Clone, Default)]
pub struct TopologyRegistry {
    proxy_factories: Arc<HashMap<String, Arc<ProxyFactory>>>,
    trace_factories: Arc<HashMap<String, Arc<TraceFactory>>>,
}

pub struct ResolvedTopology {
    pub proxy_components: Vec<ProxyComponentInstance>,
    pub trace_writers: Vec<TraceWriterInstance>,
}

impl TopologyRegistry {
    pub fn builder() -> TopologyRegistryBuilder {
        TopologyRegistryBuilder::default()
    }

    pub fn build(&self, topology: &TopologySpec) -> Result<ResolvedTopology> {
        let mut proxy_components = Vec::new();
        let mut trace_writers = Vec::new();

        for component in &topology.components {
            if let Some(factory) = self.proxy_factories.get(&component.name) {
                proxy_components.push(factory(component.config.as_ref())?);
                continue;
            }

            if let Some(factory) = self.trace_factories.get(&component.name) {
                trace_writers.push(factory(component.config.as_ref())?);
                continue;
            }

            return Err(anyhow!("unknown topology component '{}'", component.name));
        }

        Ok(ResolvedTopology {
            proxy_components,
            trace_writers,
        })
    }
}

#[derive(Default)]
pub struct TopologyRegistryBuilder {
    proxy_factories: HashMap<String, Arc<ProxyFactory>>,
    trace_factories: HashMap<String, Arc<TraceFactory>>,
}

impl TopologyRegistryBuilder {
    pub fn register_component<F>(mut self, name: impl Into<String>, factory: F) -> Self
    where
        F: Fn(Option<&Value>) -> Result<ProxyComponentInstance> + Send + Sync + 'static,
    {
        self.proxy_factories.insert(name.into(), Arc::new(factory));
        self
    }

    pub fn register_tracer<F>(mut self, name: impl Into<String>, factory: F) -> Self
    where
        F: Fn(Option<&Value>) -> Result<TraceWriterInstance> + Send + Sync + 'static,
    {
        self.trace_factories.insert(name.into(), Arc::new(factory));
        self
    }

    pub fn build(self) -> TopologyRegistry {
        TopologyRegistry {
            proxy_factories: Arc::new(self.proxy_factories),
            trace_factories: Arc::new(self.trace_factories),
        }
    }
}
