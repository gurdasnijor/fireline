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

use sacp::{Agent, Client, Conductor, DynConnectTo};
use sacp_conductor::{ConductorImpl, McpBridgeMode, trace::WriteEvent};
use sacp_tokio::AcpAgent;

/// Build a conductor whose terminal component is an ACP subprocess.
pub fn build_subprocess_conductor(
    name: impl ToString,
    agent_command: Vec<String>,
    components: Vec<DynConnectTo<Conductor>>,
    trace_writer: impl WriteEvent,
) -> ConductorImpl<Agent> {
    ConductorImpl::new_agent(
        name,
        move |req| async move {
            let terminal = DynConnectTo::<Client>::new(
                AcpAgent::from_args(agent_command)
                    .map_err(|e| sacp::util::internal_error(format!("agent command: {e}")))?,
            );
            Ok((req, components, terminal))
        },
        McpBridgeMode::default(),
    )
    .trace_to(trace_writer)
}
