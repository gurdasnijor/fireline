#![forbid(unsafe_code)]

pub mod primitive;
pub mod provider_dispatcher;
pub mod provider_model;
pub mod provider;
pub mod providers;
pub mod satisfiers;
pub mod stream_trace;

#[cfg(feature = "microsandbox-provider")]
pub mod microsandbox;

pub use provider_dispatcher::ProviderDispatcher;
pub use provider_model::{
    ExecutionResult, ProviderCapabilities, SandboxConfig, SandboxDescriptor, SandboxHandle,
    SandboxProvider, SandboxStatus,
};
pub use fireline_resources::{
    LocalPathMounter, MountedResource, ResourceMounter, ResourceRef, prepare_resources,
};
pub use fireline_session::{StreamStorageConfig, StreamStorageMode};
#[cfg(feature = "microsandbox-provider")]
pub use microsandbox::{MICROSANDBOX_SANDBOX_KIND, MicrosandboxSandbox, MicrosandboxSandboxConfig};
pub use primitive::{Sandbox, SandboxHandle as ToolSandboxHandle, ToolCall, ToolCallResult};
pub use fireline_session::{
    Endpoint, HeartbeatMetrics, HeartbeatReport, HostDescriptor, HostRegistration, HostStatus,
    PersistedHostSpec, ProvisionSpec, SandboxProviderKind, SandboxProviderRequest,
};
pub use provider::ManagedSandbox;
pub use providers::{
    DockerProvider, DockerProviderConfig, LocalSubprocessProvider, LocalSubprocessProviderConfig,
};
