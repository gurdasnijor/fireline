//! The [`Orchestrator`] primitive — the substrate-agnostic wake loop.
//!
//! An `Orchestrator` drives one or more [`crate::primitives::Host`]
//! implementations forward by calling [`crate::primitives::Host::wake`]
//! for sessions that need it. It is *Host-independent* by design: it
//! only knows how to look up sessions needing wake, dispatch a wake
//! call with retry, and coalesce concurrent wakes for the same session.
//!
//! Concrete satisfiers (a `while_loop` scheduler, a cron-driven
//! scheduler, an HTTP-triggered scheduler, …) will live in separate
//! modules or crates and implement this trait. None are provided in
//! this module yet — this file establishes the contract; Tier 4 of the
//! split proposal (see `docs/proposals/runtime-host-split.md` §7.5)
//! wires up the first satisfier against the
//! [`@fireline/state`](../../../../packages/state) session registry.

use async_trait::async_trait;

/// Drives sessions forward through repeated [`crate::primitives::Host::wake`]
/// calls.
///
/// Implementations are responsible for:
/// - finding sessions that need wake (via whatever `SessionRegistry`
///   abstraction they consume — stream-backed, HTTP-polled, queue-driven);
/// - coalescing concurrent [`Orchestrator::wake_one`] calls for the
///   same session id into a single in-flight [`crate::primitives::Host::wake`];
/// - retry + backoff when [`crate::primitives::Host::wake`] errors.
///
/// Implementations are *not* responsible for knowing which [`crate::primitives::Host`]
/// satisfier a given session belongs to — that routing is carried by
/// the opaque [`crate::primitives::SessionHandle::kind`] tag and
/// resolved in the wake handler the orchestrator is constructed with.
#[async_trait]
pub trait Orchestrator: Send + Sync {
    /// Queue a wake for a specific session. Retry-safe: multiple
    /// concurrent calls for the same `session_id` should coalesce into
    /// a single in-flight wake against the underlying host.
    async fn wake_one(&self, session_id: &str) -> anyhow::Result<()>;

    /// Begin the scheduling loop. Returns when the loop is fully
    /// running (not when it exits).
    async fn start(&self) -> anyhow::Result<()>;

    /// Stop the scheduling loop. Pending wakes may still complete but
    /// no new wakes are dispatched.
    async fn stop(&self) -> anyhow::Result<()>;
}
