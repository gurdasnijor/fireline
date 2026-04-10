//! Conductor builder.
//!
//! [`build_subprocess_conductor`] composes injected components and a
//! trace writer into a running [`sacp_conductor::ConductorImpl`].
//! The agent is spawned as a subprocess via
//! [`sacp_tokio::AcpAgent::from_args`] and becomes the terminal
//! component of the chain.
//!
//! Per the architecture: components are passed in by the caller, not
//! hard-coded here. The binary's bootstrap composes the proxy chain
//! it wants and hands it to this function.

// TODO: implement build_subprocess_conductor
//
// Target signature:
//
// ```rust,ignore
// pub fn build_subprocess_conductor(
//     name: impl ToString,
//     agent_command: Vec<String>,
//     components: Vec<sacp::DynComponent<sacp::ProxyToConductor>>,
//     trace_writer: impl sacp_conductor::trace::WriteEvent + Send + 'static,
// ) -> sacp_conductor::ConductorImpl<sacp::Agent>;
// ```
//
// See `docs/architecture.md` § "fireline-conductor" for the design intent.
