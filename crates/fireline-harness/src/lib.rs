#![forbid(unsafe_code)]

pub mod approval;
pub mod audit;
pub mod budget;
pub mod context;
pub mod routes_acp;
mod runtime_topology;
pub mod secrets;
pub mod shared_terminal;
pub mod state_projector;
pub mod topology;
pub mod trace;

pub use approval::{
    ApprovalAction, ApprovalConfig, ApprovalGateComponent, ApprovalMatch, ApprovalPolicy,
};
pub use audit::{AuditConfig, AuditDirection, AuditRecord, AuditSink, AuditTracer};
pub use budget::{BudgetAction, BudgetComponent, BudgetConfig};
pub use context::{
    ContextConfig, ContextInjectionComponent, ContextPlacement, ContextSource, DatetimeSource,
    WorkspaceFileSource,
};
pub use secrets::{
    CredentialResolver, CredentialResolverError, InjectionRule, InjectionScope, InjectionTarget,
    LocalCredentialResolver, SecretValue, SecretsInjectionComponent,
};
pub use routes_acp::{AcpRouteState, BaseComponentsFactory};
pub use shared_terminal::{AttachError, SharedTerminal, SharedTerminalAttachment};
pub use topology::{
    ComponentContext, ProxyComponentInstance, ResolvedTopology, TopologyComponentSpec,
    TopologyRegistry, TopologyRegistryBuilder, TopologySpec, TraceWriterInstance,
    audit_stream_names, build_runtime_topology_registry, ensure_named_streams,
};
pub use trace::{
    BoxedTraceWriter, CompositeTraceWriter, DurableStreamTracer, emit_host_endpoints_persisted,
    emit_runtime_instance_started, emit_runtime_instance_stopped, emit_host_spec_persisted,
};
