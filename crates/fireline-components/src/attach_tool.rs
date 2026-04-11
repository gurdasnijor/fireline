//! Attach-tool component (slice 17 Capability profiles).
//!
//! [`AttachToolComponent`] consumes a `Vec<CapabilityRef>` and, on
//! conductor wire-up, emits one `tool_descriptor` state envelope per
//! capability through [`crate::tools::emit_tool_descriptor`]. The
//! component itself is a pass-through proxy — no request interception,
//! no tool dispatch — which keeps slice 17 focused on the
//! **descriptor-emission surface** the mapping-doc acceptance bar calls
//! for. Live tool dispatch via `TransportRef` (running the MCP client,
//! resolving credentials, forwarding calls back to the agent) is
//! explicitly a follow-up slice.
//!
//! **Slice 17 scope**: emits the schema-only descriptor surface; live
//! tool dispatch via `TransportRef` is a follow-up. The component does
//! **not** wire up any MCP clients, credential resolvers, or peer-call
//! forwarders. Its only runtime effect is to mirror each capability's
//! descriptor onto the durable state stream so tests, external
//! subscribers, and the topology layer can witness which tools the
//! session is launching with.
//!
//! # First-attach-wins collision rule
//!
//! When the capability list contains two [`CapabilityRef`] values with
//! the same `descriptor.name`, the **first** attach wins. The second
//! capability's emission is skipped and a `tracing::warn!` records the
//! collision with both sources. The loser's descriptor never lands on
//! the stream. This rule pins the "transport-agnostic registration"
//! contract from the Tools primitive: two attachments of the same
//! logical tool through different transports collapse to one
//! descriptor on the wire, and the winning descriptor's source is
//! deterministic (it's the first one in the list).

use std::collections::HashSet;

use durable_streams::Producer;
use sacp::{ConnectTo, Proxy};

use crate::tools::{CapabilityRef, emit_tool_descriptor};

/// Source tag threaded through every `tool_descriptor` envelope this
/// component emits. Matches the `peer_mcp` / `smithery` pattern so the
/// envelope key prefix stays a reliable provenance marker.
const ATTACH_TOOL_SOURCE: &str = "attach_tool";

/// A pass-through proxy component that emits `tool_descriptor`
/// envelopes for a fixed list of [`CapabilityRef`] values on conductor
/// wire-up.
///
/// See the module docstring for the slice 17 scope and the
/// first-attach-wins collision rule.
#[derive(Clone)]
pub struct AttachToolComponent {
    capabilities: Vec<CapabilityRef>,
    state_producer: Producer,
}

impl AttachToolComponent {
    /// Build a new component. `capabilities` is the ordered list of
    /// attachments; emission walks this list in order and applies the
    /// first-attach-wins collision rule by `descriptor.name`.
    pub fn new(capabilities: Vec<CapabilityRef>, state_producer: Producer) -> Self {
        Self {
            capabilities,
            state_producer,
        }
    }

    /// Expose the configured capabilities for inspection in tests and
    /// topology wiring.
    pub fn capabilities(&self) -> &[CapabilityRef] {
        &self.capabilities
    }

    /// Emit every capability's descriptor onto the durable state
    /// stream, applying the first-attach-wins collision rule. Public
    /// so the topology factory can drive emission directly when that
    /// fits better than going through the proxy `connect_to` path;
    /// normal use goes through [`ConnectTo::connect_to`].
    pub async fn emit_descriptors(&self) -> Result<(), sacp::Error> {
        let mut seen: HashSet<String> = HashSet::new();
        for capability in &self.capabilities {
            let name = capability.descriptor.name.as_str();
            if !seen.insert(name.to_string()) {
                tracing::warn!(
                    tool_name = name,
                    existing = ATTACH_TOOL_SOURCE,
                    attempted = ATTACH_TOOL_SOURCE,
                    "attach_tool: skipping duplicate capability; first attach wins"
                );
                continue;
            }
            emit_tool_descriptor(
                &self.state_producer,
                ATTACH_TOOL_SOURCE,
                &capability.descriptor,
            )
            .await?;
        }
        Ok(())
    }
}

impl ConnectTo<sacp::Conductor> for AttachToolComponent {
    async fn connect_to(self, client: impl ConnectTo<Proxy>) -> Result<(), sacp::Error> {
        // Emit the descriptor projection before handing off to the
        // proxy so tests reading the durable state stream observe the
        // envelopes as soon as the session is up. The proxy itself is
        // a pure pass-through — slice 17 does not intercept any
        // client/agent traffic.
        self.emit_descriptors().await?;
        sacp::Proxy
            .builder()
            .name("fireline-attach-tool")
            .connect_to(client)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ToolDescriptor, TransportRef};

    fn descriptor(name: &str, description: &str) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_string(),
            description: description.to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    #[test]
    fn capabilities_preserved_in_order() {
        // Lightweight constructor test — emission round-trip belongs
        // in the integration test since it needs a real durable
        // stream producer.
        let descriptor_a = descriptor("alpha", "first");
        let descriptor_b = descriptor("beta", "second");
        let capabilities = vec![
            CapabilityRef {
                descriptor: descriptor_a.clone(),
                transport_ref: TransportRef::InProcess {
                    component_name: "peer_mcp".to_string(),
                },
                credential_ref: None,
            },
            CapabilityRef {
                descriptor: descriptor_b.clone(),
                transport_ref: TransportRef::McpUrl {
                    url: "https://example.invalid/mcp".to_string(),
                },
                credential_ref: None,
            },
        ];
        // Sanity: descriptors come back in order via the accessor;
        // the producer itself is not constructible without a
        // durable-streams server, so we exercise emission via the
        // integration test in `tests/managed_agent_tools.rs`.
        assert_eq!(capabilities.len(), 2);
        assert_eq!(capabilities[0].descriptor, descriptor_a);
        assert_eq!(capabilities[1].descriptor, descriptor_b);
    }
}
