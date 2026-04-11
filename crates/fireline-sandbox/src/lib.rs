#![forbid(unsafe_code)]

pub mod primitive;
pub mod satisfiers;

#[cfg(feature = "microsandbox-provider")]
pub mod microsandbox;

pub use primitive::{Sandbox, SandboxHandle, ToolCall, ToolCallResult};
#[cfg(feature = "microsandbox-provider")]
pub use microsandbox::{MicrosandboxSandbox, MicrosandboxSandboxConfig, MICROSANDBOX_SANDBOX_KIND};
