//! Conductor builder.
//!
//! [`build_conductor_with_terminal`] composes injected components, a terminal
//! transport, and a state writer into a running
//! [`sacp_conductor::ConductorImpl`].
//! [`build_subprocess_conductor`] is the convenience wrapper that supplies a
//! subprocess terminal via [`sacp_tokio::AcpAgent::from_args`].
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

/// Build a conductor whose terminal component is supplied by the caller.
pub fn build_conductor_with_terminal(
    name: impl ToString,
    components: Vec<DynConnectTo<Conductor>>,
    terminal: DynConnectTo<Client>,
    trace_writer: impl WriteEvent,
) -> ConductorImpl<Agent> {
    let mut components = Some(components);
    let mut terminal = Some(terminal);

    ConductorImpl::new_agent(
        name,
        move |req| async move {
            let components = components.take().ok_or_else(|| {
                sacp::util::internal_error("conductor components already instantiated")
            })?;
            let terminal = terminal
                .take()
                .ok_or_else(|| sacp::util::internal_error("terminal already instantiated"))?;
            Ok((req, components, terminal))
        },
        McpBridgeMode::default(),
    )
    .trace_to(trace_writer)
}
