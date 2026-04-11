//! Tools primitive: Anthropic-shaped tool descriptors.
//!
//! This module gives Fireline a **typed, Anthropic-native** projection of
//! the tools a runtime exposes to the agent. The descriptor is the
//! schema-only triple `{name, description, input_schema}` the Anthropic
//! Tools primitive specifies — no transport, no credentials, no host or
//! runtime identifiers. Transport and auth resolve inside the conductor
//! layer at call time; the agent only ever sees this triple.
//!
//! # Why a separate state envelope
//!
//! The ACP wire protocol carries tools inside MCP servers attached via
//! `NewSessionRequest.mcp_servers`. The initialize/new-session response
//! shapes do **not** include an inspectable `available_tools` field, and
//! the SDK's `McpServer` keeps its registered tools in a private
//! `HashMap` with no Rust accessor. That leaves tests (and external
//! subscribers) with no way to witness the tool triple Fireline is
//! actually registering.
//!
//! The simplest fix that keeps the MCP wire protocol untouched is to
//! mirror every registered tool as a `tool_descriptor` entity on the
//! durable state stream when the conductor wires the owning component
//! into the runtime topology. Tests then read the stream back via the
//! normal `read_all_events` oracle and assert that every envelope's
//! value contains exactly the Anthropic triple and nothing transport-
//! flavored. External subscribers get the same view for free.
//!
//! The envelope shape matches the pattern used by
//! [`crate::approval`] and [`crate::fs_backend`]:
//! `{type, key, headers.operation, value}`. Operation is always
//! `"insert"`; duplicate inserts with the same key project to the same
//! record, so repeated wiring of the same component is safe.

use durable_streams::Producer;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The Anthropic-shape tool triple.
///
/// Every tool Fireline exposes to the agent is fully described by this
/// struct. Transport details (MCP server URL, peer runtime id, host-tool
/// component name) and credential references (env var names, secret
/// store paths, OAuth scopes) live in the conductor layer and are
/// **deliberately** excluded — the schema-only contract is what makes a
/// tool portable across transports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolDescriptor {
    /// The tool name the agent uses to invoke it, e.g. `list_peers`.
    pub name: String,
    /// Human-readable description surfaced to the agent alongside the
    /// schema.
    pub description: String,
    /// JSON Schema describing the tool's input shape. Produced from
    /// the Rust input type via `schemars::schema_for!(...)` at the
    /// MCP-server wire-up site, serialized as a plain
    /// `serde_json::Value` here so downstream consumers don't need a
    /// schemars dependency.
    pub input_schema: Value,
}

/// Emit a single `ToolDescriptor` as a durable state envelope and flush
/// the producer.
///
/// `source` identifies the component that owns the tool (for example
/// `"peer_mcp"` or `"smithery"`). The envelope key is
/// `"{source}:{tool_name}"` so that repeated inserts from the same
/// source project to the same record, and so that a test can assert
/// provenance without parsing the value.
pub async fn emit_tool_descriptor(
    producer: &Producer,
    source: &str,
    descriptor: &ToolDescriptor,
) -> Result<(), sacp::Error> {
    producer.append_json(&StateEnvelope {
        entity_type: "tool_descriptor",
        key: format!("{source}:{}", descriptor.name),
        headers: StateHeaders {
            operation: "insert",
        },
        value: descriptor,
    });
    producer
        .flush()
        .await
        .map_err(|error| sacp::util::internal_error(format!("tool_descriptor flush: {error}")))
}

/// Emit a batch of `ToolDescriptor`s and flush once at the end.
///
/// Convenience over calling [`emit_tool_descriptor`] in a loop — the
/// flush is the expensive step, so batching keeps wire-up cheap when a
/// single component registers several tools (e.g. `peer_mcp` exposes
/// both `list_peers` and `prompt_peer`).
pub async fn emit_tool_descriptors(
    producer: &Producer,
    source: &str,
    descriptors: &[ToolDescriptor],
) -> Result<(), sacp::Error> {
    for descriptor in descriptors {
        producer.append_json(&StateEnvelope {
            entity_type: "tool_descriptor",
            key: format!("{source}:{}", descriptor.name),
            headers: StateHeaders {
                operation: "insert",
            },
            value: descriptor,
        });
    }
    producer
        .flush()
        .await
        .map_err(|error| sacp::util::internal_error(format!("tool_descriptor flush: {error}")))
}

#[derive(Debug, Clone, Serialize)]
struct StateHeaders {
    operation: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct StateEnvelope<'a, T: Serialize> {
    #[serde(rename = "type")]
    entity_type: &'static str,
    key: String,
    headers: StateHeaders,
    value: &'a T,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_serializes_with_camel_case_input_schema() {
        let descriptor = ToolDescriptor {
            name: "list_peers".to_string(),
            description: "List running peers".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        };
        let serialized = serde_json::to_value(&descriptor).unwrap();
        assert_eq!(serialized["name"], "list_peers");
        assert_eq!(serialized["description"], "List running peers");
        assert_eq!(serialized["inputSchema"], serde_json::json!({"type": "object"}));
        // No extra keys that would leak transport or credentials.
        let obj = serialized.as_object().unwrap();
        assert_eq!(obj.len(), 3, "expected only the Anthropic triple, got {obj:?}");
    }
}
