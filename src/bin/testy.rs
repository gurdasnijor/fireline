//! Minimal test ACP agent for integration tests.
//!
//! Speaks ACP over stdio. Responds to `initialize`, `session/new`,
//! and `session/prompt` with the simplest valid responses. Used by
//! integration tests as a deterministic stand-in for a real agent
//! (so tests don't depend on `npx`, `claude-acp`, or external
//! services).

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // TODO: implement minimal stdio ACP test agent
    //
    // Target shape:
    //
    // ```rust,ignore
    // use sacp::{Agent, Component, ByteStreams};
    // use sacp::schema::{InitializeResponse, NewSessionResponse, PromptResponse, StopReason};
    //
    // // Implement Component<sacp::Client> for a minimal TestAgent
    // // that responds with valid empty responses to each request,
    // // then call .serve() over stdio.
    // ```

    Ok(())
}
