#![forbid(unsafe_code)]

pub mod approval;
pub mod audit;
pub mod budget;
pub mod context;
pub mod topology;

pub use approval::{ApprovalAction, ApprovalConfig, ApprovalGateComponent, ApprovalMatch, ApprovalPolicy};
pub use audit::{AuditConfig, AuditDirection, AuditRecord, AuditSink, AuditTracer};
pub use budget::{BudgetAction, BudgetComponent, BudgetConfig};
pub use context::{
    ContextConfig, ContextInjectionComponent, ContextPlacement, ContextSource, DatetimeSource,
    WorkspaceFileSource,
};
pub use topology::{
    ProxyComponentInstance, ResolvedTopology, TopologyComponentSpec, TopologyRegistry,
    TopologyRegistryBuilder, TopologySpec, TraceWriterInstance,
};
