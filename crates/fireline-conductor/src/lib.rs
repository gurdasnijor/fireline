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
//!    implementation that forwards [`sacp_conductor::trace::TraceEvent`]s into
//!    the Fireline state projector and appends the resulting
//!    `STATE-PROTOCOL` changes to a durable stream.
//!
//! 3. [`transports`] — a set of feature-gated transport adapters
//!    for listener-style hosting and in-memory testing. Stdio attach
//!    uses [`sacp_tokio::Stdio`] directly rather than a Fireline wrapper.
//!
//! See [`docs/architecture.md`](../../../docs/architecture.md) for the
//! full architectural context.

#![forbid(unsafe_code)]

pub mod build;
pub mod runtime;
pub mod session;
pub mod shared_terminal;
pub mod topology;
pub mod trace;
pub mod transports;

mod state_projector;
