//! Context injection proxy — SKETCH.
//!
//! Intended shape: an inbound transformer on `session/prompt` that
//! gathers context from a list of [`ContextSource`]s and prepends
//! the assembled text to the prompt's content blocks before
//! forwarding to the agent.
//!
//! # SKETCH STATUS
//!
//! - Config, `ContextSource` trait, and the built-in
//!   [`DatetimeSource`] are implemented and tested.
//! - The `ConnectTo<Conductor>` impl is a pass-through proxy — the
//!   actual `session/prompt` interception that mutates the request's
//!   `ContentBlock::Text` array is TODO. The intended cookbook
//!   pattern is `cx.send_request_to(Agent, modified).forward_response_to(responder)`
//!   inside `on_receive_request_from(Client, SessionPromptRequest, ...)`.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use sacp::{ConnectTo, Proxy};

/// A pluggable source of per-session context text.
#[async_trait]
pub trait ContextSource: Send + Sync {
    async fn gather(&self, session_id: &str) -> Result<String, sacp::Error>;
}

#[derive(Clone, Default)]
pub struct ContextConfig {
    pub sources: Vec<Arc<dyn ContextSource>>,
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
    /// the results with `\n\n` separators. This is the piece the
    /// `session/prompt` interceptor would call before rewriting the
    /// request's prompt text.
    pub async fn assemble_context(&self, session_id: &str) -> Result<String, sacp::Error> {
        let mut parts = Vec::with_capacity(self.config.sources.len());
        for source in &self.config.sources {
            parts.push(source.gather(session_id).await?);
        }
        Ok(parts.join("\n\n"))
    }
}

impl ConnectTo<sacp::Conductor> for ContextInjectionComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        let _this = self;
        // TODO: intercept `sacp::schema::SessionPromptRequest` via
        //   .on_receive_request_from(
        //       Client,
        //       async move |request, responder, cx| {
        //           let prefix = _this.assemble_context(request.session_id()).await?;
        //           let mut request = request;
        //           prepend_text_block(&mut request, &prefix);
        //           cx.send_request_to(Agent, request).forward_response_to(responder)
        //       },
        //       sacp::on_receive_request!(),
        //   )
        // until the exact `SessionPromptRequest` field path and the
        // `send_request_to(...).forward_response_to(...)` chain are
        // pinned in this crate.
        sacp::Proxy
            .builder()
            .name("fireline-context")
            .connect_to(client)
            .await
    }
}

/// Trivial built-in source: injects the current UNIX time.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn datetime_source_returns_non_empty() {
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
        };
        let component = ContextInjectionComponent::new(config);
        let assembled = component.assemble_context("sess-1").await.unwrap();
        // Two sources joined by the "\n\n" separator -> three newlines
        // (one trailing on first, one leading on second, plus the join)
        assert!(assembled.contains("\n\n"));
    }
}
