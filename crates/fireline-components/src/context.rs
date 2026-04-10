//! Context injection proxy.
//!
//! Intercepts `session/prompt` requests flowing from the client
//! toward the agent, gathers per-session context from a list of
//! [`ContextSource`]s, and prepends the assembled text to the
//! prompt's content blocks as a new [`ContentBlock::Text`] entry.
//! Every other ACP message passes through unchanged via the
//! proxy's default-forwarding behavior.
//!
//! # Pattern
//!
//! This follows the canonical proxy interception pattern from
//! `sacp::concepts::proxies`:
//!
//! ```ignore
//! Proxy.builder()
//!     .on_receive_request_from(Client, async |req: PromptRequest, responder, cx| {
//!         let modified = /* mutate req */;
//!         cx.send_request_to(Agent, modified).forward_response_to(responder)
//!     }, sacp::on_receive_request!())
//!     .connect_to(transport)
//!     .await
//! ```
//!
//! # Sources
//!
//! A [`ContextSource`] is an async trait returning a string of
//! context text for a given session. The crate ships two built-in
//! sources:
//!
//! - [`DatetimeSource`] — prepends the current UNIX time
//! - [`WorkspaceFileSource`] — reads a file and returns its contents
//!
//! Applications add their own `impl ContextSource` for memory,
//! operator instructions, prior-session summaries, etc.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use sacp::schema::{ContentBlock, PromptRequest};
use sacp::{Agent, Client, ConnectTo, Proxy};

/// A pluggable source of per-session context text.
///
/// Implementations should return quickly — they run on the
/// `session/prompt` hot path. Expensive lookups belong in a
/// background warmer with the source reading from cache.
#[async_trait]
pub trait ContextSource: Send + Sync {
    async fn gather(&self, session_id: &str) -> Result<String, sacp::Error>;
}

#[derive(Clone, Default)]
pub struct ContextConfig {
    pub sources: Vec<Arc<dyn ContextSource>>,
    /// When the prepended context should go into the prompt.
    /// Default: as a leading text block before the user's content.
    pub placement: ContextPlacement,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ContextPlacement {
    /// Prepend one `ContentBlock::Text` containing the joined
    /// context before the user's content.
    #[default]
    Prepend,
    /// Append one `ContentBlock::Text` containing the joined
    /// context after the user's content.
    Append,
}

#[derive(Clone)]
pub struct ContextInjectionComponent {
    config: ContextConfig,
}

impl ContextInjectionComponent {
    pub fn new(config: ContextConfig) -> Self {
        Self { config }
    }

    /// Gather context from every configured source and concatenate
    /// the results with blank-line separators. Returns the empty
    /// string if no sources are configured.
    pub async fn assemble_context(&self, session_id: &str) -> Result<String, sacp::Error> {
        let mut parts = Vec::with_capacity(self.config.sources.len());
        for source in &self.config.sources {
            let text = source.gather(session_id).await?;
            if !text.is_empty() {
                parts.push(text);
            }
        }
        Ok(parts.join("\n\n"))
    }

    /// Produce a rewritten prompt by inserting the assembled context
    /// block according to the configured placement. Exposed publicly
    /// so callers (and the unit tests) can exercise the rewrite
    /// logic without a live ACP connection.
    pub fn rewrite_prompt(&self, mut request: PromptRequest, context_text: String) -> PromptRequest {
        if context_text.is_empty() {
            return request;
        }
        let context_block: ContentBlock = context_text.into();
        match self.config.placement {
            ContextPlacement::Prepend => {
                let mut new_prompt = Vec::with_capacity(request.prompt.len() + 1);
                new_prompt.push(context_block);
                new_prompt.append(&mut request.prompt);
                request.prompt = new_prompt;
            }
            ContextPlacement::Append => {
                request.prompt.push(context_block);
            }
        }
        request
    }
}

impl ConnectTo<sacp::Conductor> for ContextInjectionComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let this = Arc::new(self);
        sacp::Proxy
            .builder()
            .name("fireline-context")
            .on_receive_request_from(
                Client,
                {
                    let this = this.clone();
                    async move |request: PromptRequest, responder, cx| {
                        let session_id = request.session_id.to_string();
                        let context_text = this.assemble_context(&session_id).await?;
                        let rewritten = this.rewrite_prompt(request, context_text);
                        cx.send_request_to(Agent, rewritten)
                            .forward_response_to(responder)
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}

// ============================================================================
// Built-in sources
// ============================================================================

/// Trivial built-in: injects the current UNIX time.
pub struct DatetimeSource;

#[async_trait]
impl ContextSource for DatetimeSource {
    async fn gather(&self, _session_id: &str) -> Result<String, sacp::Error> {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Ok(format!("[context:datetime] current unix time: {secs}"))
    }
}

/// Reads a file from disk and returns its contents verbatim.
///
/// Typical usage is to point at a workspace-scoped instruction file
/// like `CLAUDE.md` or a project `AGENTS.md`. If the file is absent
/// or unreadable the source returns an empty string rather than
/// failing the request — the goal is "soft context," not a hard
/// dependency of the prompt path.
pub struct WorkspaceFileSource {
    path: PathBuf,
    label: String,
}

impl WorkspaceFileSource {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let label = path
            .file_name()
            .map(|os| os.to_string_lossy().into_owned())
            .unwrap_or_else(|| "workspace-file".to_string());
        Self { path, label }
    }
}

#[async_trait]
impl ContextSource for WorkspaceFileSource {
    async fn gather(&self, _session_id: &str) -> Result<String, sacp::Error> {
        match tokio::fs::read_to_string(&self.path).await {
            Ok(contents) if !contents.is_empty() => {
                Ok(format!("[context:{}]\n{contents}", self.label))
            }
            _ => Ok(String::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn datetime_source_is_non_empty() {
        let source = DatetimeSource;
        let text = source.gather("sess").await.unwrap();
        assert!(text.contains("current unix time"));
    }

    #[tokio::test]
    async fn assemble_concatenates_sources() {
        let config = ContextConfig {
            sources: vec![
                Arc::new(DatetimeSource) as Arc<dyn ContextSource>,
                Arc::new(DatetimeSource) as Arc<dyn ContextSource>,
            ],
            placement: ContextPlacement::Prepend,
        };
        let component = ContextInjectionComponent::new(config);
        let assembled = component.assemble_context("sess-1").await.unwrap();
        assert!(assembled.contains("\n\n"));
    }

    #[tokio::test]
    async fn assemble_skips_empty_sources() {
        struct EmptySource;
        #[async_trait]
        impl ContextSource for EmptySource {
            async fn gather(&self, _: &str) -> Result<String, sacp::Error> {
                Ok(String::new())
            }
        }

        let config = ContextConfig {
            sources: vec![
                Arc::new(EmptySource) as Arc<dyn ContextSource>,
                Arc::new(DatetimeSource) as Arc<dyn ContextSource>,
            ],
            placement: ContextPlacement::Prepend,
        };
        let component = ContextInjectionComponent::new(config);
        let assembled = component.assemble_context("sess-1").await.unwrap();
        // No leading blank-line separator from the empty source.
        assert!(!assembled.starts_with("\n\n"));
        assert!(assembled.contains("current unix time"));
    }

    #[test]
    fn rewrite_prompt_prepends_context_block() {
        let component = ContextInjectionComponent::new(ContextConfig::default());
        let original = PromptRequest::new(
            sacp::schema::SessionId::from("sess-1"),
            vec![ContentBlock::from("user message".to_string())],
        );
        let rewritten = component.rewrite_prompt(original, "prefix context".to_string());
        assert_eq!(rewritten.prompt.len(), 2);
        match &rewritten.prompt[0] {
            ContentBlock::Text(text) => assert_eq!(text.text, "prefix context"),
            other => panic!("expected Text block, got {other:?}"),
        }
        match &rewritten.prompt[1] {
            ContentBlock::Text(text) => assert_eq!(text.text, "user message"),
            other => panic!("expected Text block, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_prompt_append_places_context_last() {
        let component = ContextInjectionComponent::new(ContextConfig {
            placement: ContextPlacement::Append,
            ..Default::default()
        });
        let original = PromptRequest::new(
            sacp::schema::SessionId::from("sess-1"),
            vec![ContentBlock::from("user message".to_string())],
        );
        let rewritten = component.rewrite_prompt(original, "postfix context".to_string());
        assert_eq!(rewritten.prompt.len(), 2);
        match &rewritten.prompt[1] {
            ContentBlock::Text(text) => assert_eq!(text.text, "postfix context"),
            other => panic!("expected Text block, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_prompt_noop_on_empty_context() {
        let component = ContextInjectionComponent::new(ContextConfig::default());
        let original = PromptRequest::new(
            sacp::schema::SessionId::from("sess-1"),
            vec![ContentBlock::from("user message".to_string())],
        );
        let rewritten = component.rewrite_prompt(original.clone(), String::new());
        assert_eq!(rewritten.prompt.len(), 1);
    }
}
