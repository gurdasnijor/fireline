//! Secrets injection policy and credential-resolution surface.
//!
//! This module defines the harness-side policy objects for resolving
//! credentials and preparing them for later injection into tool execution
//! paths. The first shipped slice is intentionally narrow:
//!
//! - session-scoped `EnvVar` injection is the only end-to-end supported path
//! - generic MCP header and tool-argument mutation remain future work
//! - resolved plaintext stays wrapped in [`SecretValue`] so callers do not
//!   accidentally serialize or log it

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fireline_tools::CredentialRef;
use zeroize::Zeroizing;

/// Harness-layer secret-resolution policy and cache.
#[derive(Clone)]
pub struct SecretsInjectionComponent {
    resolver: Arc<dyn CredentialResolver>,
    rules: Arc<[InjectionRule]>,
    session_cache: Arc<Mutex<HashMap<String, HashMap<String, Arc<SecretValue>>>>>,
    once_cache: Arc<Mutex<HashMap<usize, Arc<SecretValue>>>>,
}

impl SecretsInjectionComponent {
    pub fn new(resolver: Arc<dyn CredentialResolver>, rules: Vec<InjectionRule>) -> Self {
        Self {
            resolver,
            rules: Arc::<[InjectionRule]>::from(rules),
            session_cache: Arc::new(Mutex::new(HashMap::new())),
            once_cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn rules(&self) -> &[InjectionRule] {
        &self.rules
    }

    pub(crate) fn resolver(&self) -> &Arc<dyn CredentialResolver> {
        &self.resolver
    }

    pub(crate) fn session_cache(
        &self,
    ) -> &Arc<Mutex<HashMap<String, HashMap<String, Arc<SecretValue>>>>> {
        &self.session_cache
    }

    pub(crate) fn once_cache(&self) -> &Arc<Mutex<HashMap<usize, Arc<SecretValue>>>> {
        &self.once_cache
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectionRule {
    pub target: InjectionTarget,
    pub credential_ref: CredentialRef,
    pub scope: InjectionScope,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InjectionTarget {
    /// Set an environment variable for the target worker process.
    EnvVar(String),
    /// Attach a header to outbound requests for a named MCP server.
    McpServerHeader { server: String, header: String },
    /// Write the resolved value into a specific tool-argument path.
    ToolArg { tool: String, arg_path: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InjectionScope {
    /// Resolve once per session and reuse for the session lifetime.
    Session,
    /// Resolve every time the rule is applied.
    PerCall,
    /// Resolve once on first use and reuse until revoked.
    Once,
}

#[async_trait]
pub trait CredentialResolver: Send + Sync {
    async fn resolve(
        &self,
        credential_ref: &CredentialRef,
        session_id: &str,
    ) -> Result<SecretValue, CredentialResolverError>;
}

/// In-memory wrapper around plaintext secret material.
///
/// The inner string is zeroized on drop and never implements serialization.
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl AsRef<str> for SecretValue {
    fn as_ref(&self) -> &str {
        self.expose()
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialResolverError {
    NotFound {
        credential_ref_name: String,
    },
    Forbidden {
        credential_ref_name: String,
        reason: Option<String>,
    },
    Expired {
        credential_ref_name: String,
        expired_at_ms: Option<u64>,
    },
    Transport {
        store: &'static str,
        message: String,
    },
}

impl std::fmt::Display for CredentialResolverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound {
                credential_ref_name,
            } => write!(f, "credential '{credential_ref_name}' not found"),
            Self::Forbidden {
                credential_ref_name,
                reason,
            } => {
                write!(f, "credential '{credential_ref_name}' forbidden")?;
                if let Some(reason) = reason {
                    write!(f, ": {reason}")?;
                }
                Ok(())
            }
            Self::Expired {
                credential_ref_name,
                expired_at_ms,
            } => {
                write!(f, "credential '{credential_ref_name}' expired")?;
                if let Some(expired_at_ms) = expired_at_ms {
                    write!(f, " at {expired_at_ms}")?;
                }
                Ok(())
            }
            Self::Transport { store, message } => {
                write!(f, "{store} credential transport error: {message}")
            }
        }
    }
}

impl std::error::Error for CredentialResolverError {}

#[cfg(test)]
mod tests {
    use super::SecretValue;

    #[test]
    fn secret_value_debug_is_redacted() {
        let value = SecretValue::new("top-secret");
        assert_eq!(format!("{value:?}"), "SecretValue(<redacted>)");
    }

    #[test]
    fn secret_value_exposes_plaintext_by_reference() {
        let value = SecretValue::new("abc123");
        assert_eq!(value.expose(), "abc123");
        assert_eq!(value.as_ref(), "abc123");
    }
}
