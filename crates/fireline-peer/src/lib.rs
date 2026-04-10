//! # fireline-peer
//!
//! Cross-agent peer-call component for Fireline.
//!
//! Provides [`PeerComponent`] — a [`sacp::component::Component`]
//! implementation (`Component<sacp::ProxyToConductor>`) that:
//!
//! 1. Intercepts `NewSessionRequest` and dynamically injects an MCP
//!    server that exposes `list_peers` and `prompt_peer` tools to the
//!    agent (the same `with_mcp_server` pattern that
//!    [Sparkle](https://github.com/sparkle-ai-space/sparkle-mcp/blob/main/src/acp_component.rs)
//!    uses to inject embodiment).
//!
//! 2. Reads `_meta.parentSessionId` and `_meta.parentPromptTurnId`
//!    from incoming peer-call `initialize` requests and stores them
//!    in per-session lineage state.
//!
//! 3. When the agent calls `prompt_peer`, dispatches the call to the
//!    target peer over the configured transport, stamping
//!    `_meta.parent*` on the outgoing `initialize` so the receiving
//!    Fireline instance can record the lineage.
//!
//! 4. Forwards every other message through transparently.
//!
//! All state is owned internally by the component instance:
//!
//! - The peer directory (a file-backed list of running Fireline
//!   instances on this machine — see [`directory`]).
//! - The MCP server that's injected per session (see [`mcp_server`]).
//! - The peer transport (HTTP today, ACP-native after the spike — see
//!   [`transport`]).
//! - A small per-session lineage `HashMap` tracking which sessions are
//!   descended from cross-node peer calls.
//!
//! See [`docs/architecture.md`](../../../docs/architecture.md) for
//! the full architectural context.

#![forbid(unsafe_code)]

pub mod component;
pub mod directory;

pub(crate) mod mcp_server;
pub(crate) mod transport;

pub use component::PeerComponent;
pub use directory::Directory;
