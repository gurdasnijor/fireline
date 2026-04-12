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
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fireline_tools::CredentialRef;
use serde::Deserialize;
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

#[derive(Clone, Debug)]
pub struct LocalCredentialResolver {
    pub toml_path: PathBuf,
    pub env_fallback: bool,
}

impl Default for LocalCredentialResolver {
    fn default() -> Self {
        Self {
            toml_path: Self::default_path(),
            env_fallback: true,
        }
    }
}

impl LocalCredentialResolver {
    pub fn new(toml_path: impl Into<PathBuf>) -> Self {
        Self {
            toml_path: toml_path.into(),
            env_fallback: true,
        }
    }

    pub fn default_path() -> PathBuf {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("~"))
            .join(".config")
            .join("fireline")
            .join("secrets.toml")
    }

    fn resolve_from_file(
        &self,
        credential_ref: &CredentialRef,
    ) -> Result<Option<SecretValue>, CredentialResolverError> {
        let config = read_local_secrets_file(&self.toml_path)?;
        match credential_ref {
            CredentialRef::Env { .. } => Ok(None),
            CredentialRef::Secret { key } => Ok(config.secrets.get(key).cloned().map(SecretValue::new)),
            CredentialRef::OauthToken { provider, account } => {
                let account_name = account.as_deref().unwrap_or("default");
                Ok(config
                    .oauth
                    .get(provider)
                    .and_then(|accounts| accounts.get(account_name))
                    .cloned()
                    .map(SecretValue::new))
            }
        }
    }

    fn resolve_from_env(&self, credential_ref: &CredentialRef) -> Option<SecretValue> {
        if !self.env_fallback {
            return None;
        }

        match credential_ref {
            CredentialRef::Env { var } => std::env::var(var).ok().map(SecretValue::new),
            CredentialRef::Secret { key } => {
                std::env::var(secret_env_var_name(key)).ok().map(SecretValue::new)
            }
            CredentialRef::OauthToken { provider, account } => std::env::var(
                oauth_env_var_name(provider, account.as_deref()),
            )
            .ok()
            .map(SecretValue::new),
        }
    }
}

#[async_trait]
impl CredentialResolver for LocalCredentialResolver {
    async fn resolve(
        &self,
        credential_ref: &CredentialRef,
        _session_id: &str,
    ) -> Result<SecretValue, CredentialResolverError> {
        if let Some(value) = self.resolve_from_file(credential_ref)? {
            return Ok(value);
        }

        if let Some(value) = self.resolve_from_env(credential_ref) {
            return Ok(value);
        }

        Err(CredentialResolverError::NotFound {
            credential_ref_name: credential_ref_name(credential_ref),
        })
    }
}

#[derive(Debug, Default, Deserialize)]
struct LocalSecretsFile {
    #[serde(default)]
    secrets: HashMap<String, String>,
    #[serde(default)]
    oauth: HashMap<String, HashMap<String, String>>,
}

fn read_local_secrets_file(path: &Path) -> Result<LocalSecretsFile, CredentialResolverError> {
    match std::fs::read_to_string(path) {
        Ok(raw) => toml::from_str::<LocalSecretsFile>(&raw).map_err(|error| {
            CredentialResolverError::Transport {
                store: "local_toml",
                message: format!("parse {}: {error}", path.display()),
            }
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(LocalSecretsFile::default()),
        Err(error) => Err(CredentialResolverError::Transport {
            store: "local_toml",
            message: format!("read {}: {error}", path.display()),
        }),
    }
}

fn credential_ref_name(credential_ref: &CredentialRef) -> String {
    match credential_ref {
        CredentialRef::Env { var } => format!("env:{var}"),
        CredentialRef::Secret { key } => format!("secret:{key}"),
        CredentialRef::OauthToken { provider, account } => {
            format!("oauth:{provider}:{}", account.as_deref().unwrap_or("default"))
        }
    }
}

fn secret_env_var_name(key: &str) -> String {
    normalize_env_name(key)
}

fn oauth_env_var_name(provider: &str, account: Option<&str>) -> String {
    format!(
        "FIRELINE_OAUTH_{}_{}",
        normalize_env_name(provider),
        normalize_env_name(account.unwrap_or("default"))
    )
}

fn normalize_env_name(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut last_was_underscore = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_uppercase());
            last_was_underscore = false;
        } else if !last_was_underscore {
            normalized.push('_');
            last_was_underscore = true;
        }
    }
    normalized.trim_matches('_').to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        CredentialResolver, CredentialResolverError, LocalCredentialResolver, SecretValue,
        normalize_env_name, oauth_env_var_name, secret_env_var_name,
    };
    use fireline_tools::CredentialRef;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

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

    #[tokio::test]
    async fn local_resolver_reads_secret_from_toml() {
        let path = write_temp_file(
            r#"
[secrets]
openai_api_key = "sk-test"
"#,
        );
        let resolver = LocalCredentialResolver::new(&path);
        let value = resolver
            .resolve(
                &CredentialRef::Secret {
                    key: "openai_api_key".to_string(),
                },
                "sess-1",
            )
            .await
            .expect("secret should resolve");
        assert_eq!(value.expose(), "sk-test");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn local_resolver_reads_oauth_token_from_toml() {
        let path = write_temp_file(
            r#"
[oauth.github]
default = "gho_default"
work = "gho_work"
"#,
        );
        let resolver = LocalCredentialResolver::new(&path);
        let value = resolver
            .resolve(
                &CredentialRef::OauthToken {
                    provider: "github".to_string(),
                    account: Some("work".to_string()),
                },
                "sess-1",
            )
            .await
            .expect("oauth token should resolve");
        assert_eq!(value.expose(), "gho_work");
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn local_resolver_uses_env_variant_directly() {
        let resolver = LocalCredentialResolver::default();
        let value = resolver
            .resolve(
                &CredentialRef::Env {
                    var: "HOME".to_string(),
                },
                "sess-1",
            )
            .await
            .expect("HOME should resolve in test environment");
        assert!(!value.expose().is_empty());
    }

    #[tokio::test]
    async fn local_resolver_uses_env_fallback_for_secret_refs() {
        let path = missing_temp_path();
        let resolver = LocalCredentialResolver::new(&path);
        let value = resolver
            .resolve(
                &CredentialRef::Secret {
                    key: "home".to_string(),
                },
                "sess-1",
            )
            .await
            .expect("HOME should resolve via fallback");
        assert!(!value.expose().is_empty());
    }

    #[tokio::test]
    async fn local_resolver_returns_not_found_with_canonical_name() {
        let path = missing_temp_path();
        let resolver = LocalCredentialResolver::new(&path);
        let error = resolver
            .resolve(
                &CredentialRef::Secret {
                    key: "missing-key".to_string(),
                },
                "sess-1",
            )
            .await
            .expect_err("missing secret should not resolve");
        assert_eq!(
            error,
            CredentialResolverError::NotFound {
                credential_ref_name: "secret:missing-key".to_string(),
            }
        );
    }

    #[test]
    fn env_name_helpers_normalize_consistently() {
        assert_eq!(normalize_env_name("gh-token"), "GH_TOKEN");
        assert_eq!(secret_env_var_name("openai.api-key"), "OPENAI_API_KEY");
        assert_eq!(
            oauth_env_var_name("github-enterprise", Some("work-account")),
            "FIRELINE_OAUTH_GITHUB_ENTERPRISE_WORK_ACCOUNT"
        );
    }

    fn write_temp_file(contents: &str) -> PathBuf {
        let path = unique_temp_path();
        std::fs::write(&path, contents).expect("write temp secrets file");
        path
    }

    fn missing_temp_path() -> PathBuf {
        unique_temp_path()
    }

    fn unique_temp_path() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("fireline-secrets-{nanos}.toml"))
    }
}
