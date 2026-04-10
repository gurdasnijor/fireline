//! # fireline-conductor
//!
//! ACP conductor wiring substrate for Fireline.
//!
//! This crate provides three things:
//!
//! 1. [`build::build_subprocess_conductor`] — composes a [`Vec`] of
//!    [`sacp::DynComponent<sacp::ProxyToConductor>`] and a
//!    [`sacp_conductor::trace::WriteEvent`] into a running
//!    [`sacp_conductor::ConductorImpl`] that can be served over any
//!    transport.
//!
//! 2. [`trace::DurableStreamTracer`] — a [`sacp_conductor::trace::WriteEvent`]
//!    implementation that observes [`sacp_conductor::trace::TraceEvent`]s,
//!    correlates them into normalized `STATE-PROTOCOL` entity changes, and
//!    appends those changes to a durable stream. It is passive with respect to
//!    ACP message flow: it may read `_meta`, but active components remain
//!    responsible for stamping `_meta` extensions.
//!
//! 3. [`transports`] — a set of feature-gated transport adapters
//!    for listener-style hosting and in-memory testing. Stdio attach
//!    uses [`sacp_tokio::Stdio`] directly rather than a Fireline wrapper.
//!
//! See [`docs/architecture.md`](../../../docs/architecture.md) for the
//! full architectural context.

#![forbid(unsafe_code)]

pub mod build;
pub mod lineage;
pub mod trace;
pub mod transports;
