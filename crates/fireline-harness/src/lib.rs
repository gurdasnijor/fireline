#![forbid(unsafe_code)]

mod agent_observability;
pub mod approval;
pub mod audit;
pub mod auto_approve;
pub mod awakeable;
pub mod budget;
pub mod context;
pub mod durable_subscriber;
mod host_topology;
pub mod peer_routing;
pub mod routes_acp;
pub mod resolve_awakeable;
pub mod secrets;
pub mod shared_terminal;
pub mod state_projector;
pub mod telegram_subscriber;
pub mod topology;
pub mod trace;
pub mod webhook_subscriber;
pub mod workflow_context;

pub use agent_observability::AgentPlaneTracer;
pub use approval::{
    ApprovalAction, ApprovalConfig, ApprovalGateComponent, ApprovalMatch, ApprovalPolicy,
    approval_resolution_envelope, permission_request_envelope,
};
pub use audit::{AuditConfig, AuditDirection, AuditRecord, AuditSink, AuditTracer};
pub use auto_approve::{AutoApproveConfig, AutoApproveSubscriber, AutoApproveSubscriberComponent};
pub use awakeable::{
    AWAKEABLE_REJECTED_KIND, AWAKEABLE_RESOLVED_KIND, AWAKEABLE_WAITING_KIND, AwakeableFuture,
    AwakeableKey, AwakeableRejected, AwakeableResolved, AwakeableSubscriber, AwakeableWaiting,
    awakeable_rejection_envelope, awakeable_resolution_envelope, awakeable_waiting_envelope,
};
pub use budget::{BudgetAction, BudgetComponent, BudgetConfig};
pub use context::{
    ContextConfig, ContextInjectionComponent, ContextPlacement, ContextSource, DatetimeSource,
    WorkspaceFileSource,
};
pub use durable_subscriber::{
    ActiveSubscriber, AlwaysOnDeploymentSubscriber, CompletionKey, DEPLOYMENT_WAKE_REQUESTED_KIND,
    DeploymentWakeHandler, DeploymentWakeRequested, DurableSubscriber, DurableSubscriberDriver,
    HandlerOutcome, PassiveSubscriber, PassiveWaitPolicy, ProvisionedRuntime,
    ResumeDeploymentWakeHandler, RetryPolicy, SANDBOX_PROVISIONED_KIND, SandboxProvisioned,
    StreamEnvelope, SubscriberMode, SubscriberRegistration, SystemWakeTimerRuntime,
    TIMER_FIRED_KIND, TimerFired, TraceContext, WAKE_TIMER_REQUESTED_KIND, WakeTimerRequest,
    WakeTimerRuntime, WakeTimerSubscriber,
};
pub use peer_routing::{
    PEER_DELIVERY_ACK_ENTITY_TYPE, PeerDeliveryAcknowledged, PeerDispatchSuccess,
    PeerRoutingDispatcher, PeerRoutingEvent, PeerRoutingSubscriber,
};
pub use resolve_awakeable::{AwakeableResolver, ResolveError};
pub use routes_acp::{AcpRouteState, BaseComponentsFactory};
pub use secrets::{
    CredentialResolver, CredentialResolverError, InjectionRule, InjectionScope, InjectionTarget,
    LocalCredentialResolver, SecretValue, SecretsInjectionComponent,
};
pub use shared_terminal::{AttachError, SharedTerminal, SharedTerminalAttachment};
pub use telegram_subscriber::{
    TelegramApprovalResolution, TelegramParseMode, TelegramScope, TelegramSubscriber,
    TelegramSubscriberConfig, append_telegram_approval_resolution,
};
pub use topology::{
    ComponentContext, ProxyComponentInstance, ResolvedTopology, TopologyComponentSpec,
    TopologyRegistry, TopologyRegistryBuilder, TopologySpec, TraceWriterInstance,
    audit_stream_names, build_host_topology_registry, ensure_named_streams,
};
pub use trace::{
    BoxedTraceWriter, CompositeTraceWriter, DurableStreamTracer, emit_host_endpoints_persisted,
    emit_host_instance_started, emit_host_instance_stopped, emit_host_spec_persisted,
};
pub use webhook_subscriber::{
    DurableWebhookCursorStore, DurableWebhookDeadLetterSink, WebhookCursorRecord,
    WebhookDeadLetterRecord, WebhookDelivered, WebhookDeliveryPayload, WebhookDispatchOutcome,
    WebhookDispatchResult, WebhookEventSelector, WebhookSkipReason, WebhookSubscriber,
    WebhookSubscriberConfig, WebhookTargetConfig, append_webhook_completion,
};
pub use workflow_context::WorkflowContext;
