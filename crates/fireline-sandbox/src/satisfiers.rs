//! [`fireline_sandbox::Sandbox`] satisfiers — tool-execution
//! backends.
//!
//! This module is the home for concrete [`fireline_sandbox::Sandbox`]
//! satisfiers. Each submodule wraps one tool-execution backend (a
//! microVM, a container, a subprocess, …) and exposes it through the
//! shared [`fireline_sandbox::Sandbox`] trait so any
//! [`fireline_conductor::primitives::Host`] can delegate its tool-execution
//! surface to it.
//!
//! See `docs/proposals/runtime-host-split.md` §7 for the primitive
//! taxonomy and `docs/proposals/client-primitives.md` §Module 3 for the
//! TypeScript contract this mirrors.
//!
//! The outer `sandbox` module is always compiled so future satisfiers
//! (Docker, local subprocess, …) can land without feature-gating the
//! whole tree. Individual backends that carry heavy build-time
//! dependencies are gated behind their own feature — the microsandbox
//! satisfier in particular is behind `microsandbox-provider` because
//! its upstream deps (keyring → libdbus-sys on Linux, microsandbox-
//! prebuilt → libkrun binaries) are not buildable on default CI
//! runners.

#[cfg(feature = "microsandbox-provider")]
pub use crate::microsandbox::{
    MICROSANDBOX_SANDBOX_KIND, MicrosandboxSandbox, MicrosandboxSandboxConfig,
};
