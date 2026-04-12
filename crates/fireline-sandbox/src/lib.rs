#![forbid(unsafe_code)]

pub mod dispatcher;
pub mod primitive;
pub mod provider;
pub mod provider_trait;
pub mod providers;
pub mod registry;
pub mod satisfiers;
pub mod stream_trace;

#[cfg(feature = "microsandbox-provider")]
pub mod microsandbox;

pub use dispatcher::SandboxDispatcher;
pub use fireline_resources::{
    LocalPathMounter, MountedResource, ResourceMounter, ResourceRef, prepare_resources,
};
pub use fireline_session::{StreamStorageConfig, StreamStorageMode};
#[cfg(feature = "microsandbox-provider")]
pub use microsandbox::{MICROSANDBOX_SANDBOX_KIND, MicrosandboxSandbox, MicrosandboxSandboxConfig};
pub use primitive::{Sandbox, SandboxHandle, ToolCall, ToolCallResult};
pub use provider::{
    Endpoint, HeartbeatMetrics, HeartbeatReport, HostDescriptor, HostRegistration, HostStatus,
    ManagedSandbox, PersistedHostSpec, ProvisionSpec, SandboxLaunch, SandboxProvider,
    SandboxProviderKind, SandboxProviderRequest, SandboxTokenIssuer,
};
pub use provider_trait::LocalSandboxLauncher;
pub use providers::{DockerProvider, DockerProviderConfig, LocalProvider};
pub use registry::RuntimeRegistry;
