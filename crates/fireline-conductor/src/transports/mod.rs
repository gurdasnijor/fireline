//! Transport adapters for the conductor.
//!
//! Each transport adapter takes a duplex byte source and a built
//! [`sacp_conductor::ConductorImpl`], wraps the bytes in an
//! [`sacp::ByteStreams`], and runs the conductor over it. None of the
//! adapters know anything about the conductor's internals; they just
//! bridge a specific byte source into the SDK's transport-agnostic
//! interface.
//!
//! Each adapter is feature-gated so consumers can opt in to only the
//! transports they need. The default features include all of them.

#[cfg(feature = "transport-stdio")]
pub mod stdio;

#[cfg(feature = "transport-websocket")]
pub mod websocket;

#[cfg(feature = "transport-duplex")]
pub mod duplex;
