//! [`crate::Sandbox`][] satisfiers — tool-execution backends.
//!
//! This module is the home for concrete [`fireline_conductor::primitives::Sandbox`]
//! satisfiers. Each submodule wraps one tool-execution backend (a
//! microVM, a container, a subprocess, …) and exposes it through the
//! shared [`fireline_conductor::primitives::Sandbox`] trait so any
//! [`fireline_conductor::primitives::Host`] can delegate its tool-execution
//! surface to it.
//!
//! See `docs/proposals/runtime-host-split.md` §7 for the primitive
//! taxonomy and `docs/proposals/client-primitives.md` §Module 3 for the
//! TypeScript contract this mirrors.

pub mod microsandbox;

pub use microsandbox::{MicrosandboxSandbox, MicrosandboxSandboxConfig, MICROSANDBOX_SANDBOX_KIND};
