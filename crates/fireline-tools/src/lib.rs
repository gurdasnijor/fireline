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
//!
//! # Capability profiles (slice 17)
//!
//! Slice 17 extends the schema-only surface with a **portable
//! attachment** shape that still keeps `ToolDescriptor` honest: a
//! [`CapabilityRef`] carries a descriptor plus a [`TransportRef`] (how
//! to reach the tool at call time) and an optional [`CredentialRef`]
//! (where to resolve auth). These two refs live next to the descriptor
//! for launch-time plumbing but are **never** part of the on-wire
//! `tool_descriptor` envelope — emission through
//! [`emit_tool_descriptor`] projects only the Anthropic triple, so the
//! schema-only contract is preserved even as the capability layer
//! grows.

use durable_streams::Producer;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod agent_catalog;
pub mod attach;
pub mod peer;
pub mod smithery;

pub use attach::AttachToolComponent;
pub use peer::PeerComponent;
pub use peer::lookup;
pub use peer::stream::{
    DEFAULT_TENANT_ID, DeploymentDiscoveryEvent, DeploymentIndex, HostEntry, RuntimeEntry,
    StreamDeploymentPeerRegistry, deployment_stream_url,
};
pub use peer::{Peer, PeerRegistry};

pub mod directory {
    pub use crate::{Peer, PeerRegistry};
}

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

/// Launch-time portable reference that explains **how** a tool is
/// reached, without leaking any transport detail into the agent-visible
/// [`ToolDescriptor`].
///
/// Matches the TypeScript `TransportRef` shape documented in
/// `docs/explorations/typescript-typed-functional-core-api.md` §Tools,
/// tagged by a `kind` discriminator for ergonomic JSON round-tripping.
/// `rename_all_fields` keeps the inner variant fields in camelCase on
/// the wire so the TS side can parse them without custom decoders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TransportRef {
    /// Resolve the tool through another Fireline runtime identified by
    /// its runtime key. Used for cross-runtime peering over ACP.
    PeerRuntime { runtime_key: String },
    /// Resolve the tool through a Smithery catalog entry by namespace
    /// and tool name.
    Smithery { catalog: String, tool: String },
    /// Resolve the tool by connecting directly to a remote MCP server
    /// URL at call time.
    McpUrl { url: String },
    /// Reserved for components that export their tools through the SDK
    /// builder chain and have no standalone URL. The string identifies
    /// the component the conductor should delegate to (for example
    /// `"peer_mcp"`).
    InProcess { component_name: String },
}

/// Portable reference to the credential a tool needs at call time.
///
/// The credential itself never crosses the agent boundary — only the
/// indirection handle. Concrete resolution (env var read, secret store
/// fetch, OAuth token exchange) happens in the conductor layer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CredentialRef {
    /// Resolve at call time from a process environment variable.
    Env { var: String },
    /// Resolve at call time from a secret-store key.
    Secret { key: String },
    /// Resolve at call time from an OAuth token provider, optionally
    /// scoped to a named account.
    OauthToken {
        provider: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        account: Option<String>,
    },
}

/// A launch-time capability attachment: the Anthropic-shape
/// [`ToolDescriptor`] the agent sees paired with the portable
/// [`TransportRef`] that tells the conductor where to fetch the tool
/// and an optional [`CredentialRef`] for auth resolution.
///
/// Emission through [`emit_tool_descriptor`] projects only the
/// `descriptor` half onto the durable state stream. `transport_ref`
/// and `credential_ref` stay inside the conductor layer, which is what
/// makes this a portable capability handle rather than an agent-visible
/// API leak.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRef {
    pub descriptor: ToolDescriptor,
    pub transport_ref: TransportRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<CredentialRef>,
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
        assert_eq!(
            serialized["inputSchema"],
            serde_json::json!({"type": "object"})
        );
        // No extra keys that would leak transport or credentials.
        let obj = serialized.as_object().unwrap();
        assert_eq!(
            obj.len(),
            3,
            "expected only the Anthropic triple, got {obj:?}"
        );
    }

    fn sample_descriptor() -> ToolDescriptor {
        ToolDescriptor {
            name: "fake_tool".to_string(),
            description: "A fake tool for tests".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn assert_round_trip<T>(value: &T, expected_kind: &str)
    where
        T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let serialized = serde_json::to_value(value).unwrap();
        assert_eq!(
            serialized.get("kind").and_then(Value::as_str),
            Some(expected_kind),
            "variant must serialize with `kind` tag matching TS wire shape; got {serialized}",
        );
        let parsed: T = serde_json::from_value(serialized).unwrap();
        assert_eq!(&parsed, value);
    }

    #[test]
    fn transport_ref_field_names_are_camel_case_on_the_wire() {
        // Explicit: slice 17 pins that inner variant fields serialize
        // with camelCase on the wire so the TypeScript side can parse
        // them directly without custom decoders. This test fails loudly
        // if a future refactor drops `rename_all_fields` and the inner
        // fields revert to snake_case.
        let peer = TransportRef::PeerRuntime {
            runtime_key: "rt-1".to_string(),
        };
        let value = serde_json::to_value(&peer).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"kind": "peerRuntime", "runtimeKey": "rt-1"}),
            "TransportRef inner variant fields must serialize as camelCase, got {value}",
        );
        let inproc = TransportRef::InProcess {
            component_name: "peer_mcp".to_string(),
        };
        let value = serde_json::to_value(&inproc).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"kind": "inProcess", "componentName": "peer_mcp"}),
            "TransportRef::InProcess fields must serialize as camelCase, got {value}",
        );
    }

    #[test]
    fn transport_ref_variants_round_trip() {
        assert_round_trip(
            &TransportRef::PeerRuntime {
                runtime_key: "rt-1".to_string(),
            },
            "peerRuntime",
        );
        assert_round_trip(
            &TransportRef::Smithery {
                catalog: "@smithery".to_string(),
                tool: "notion_search".to_string(),
            },
            "smithery",
        );
        assert_round_trip(
            &TransportRef::McpUrl {
                url: "https://example.invalid/mcp".to_string(),
            },
            "mcpUrl",
        );
        assert_round_trip(
            &TransportRef::InProcess {
                component_name: "peer_mcp".to_string(),
            },
            "inProcess",
        );
    }

    #[test]
    fn credential_ref_variants_round_trip() {
        assert_round_trip(
            &CredentialRef::Env {
                var: "MY_KEY".to_string(),
            },
            "env",
        );
        assert_round_trip(
            &CredentialRef::Secret {
                key: "prod/api-token".to_string(),
            },
            "secret",
        );
        assert_round_trip(
            &CredentialRef::OauthToken {
                provider: "github".to_string(),
                account: Some("user@example.com".to_string()),
            },
            "oauthToken",
        );
        // Account optional: confirm we skip when absent.
        let tokenless = CredentialRef::OauthToken {
            provider: "github".to_string(),
            account: None,
        };
        let value = serde_json::to_value(&tokenless).unwrap();
        assert!(
            value.get("account").is_none(),
            "credential_ref.oauthToken must skip absent `account`, got {value}",
        );
        let parsed: CredentialRef = serde_json::from_value(value).unwrap();
        assert_eq!(parsed, tokenless);
    }

    #[test]
    fn capability_ref_preserves_descriptor_and_refs() {
        let capability = CapabilityRef {
            descriptor: sample_descriptor(),
            transport_ref: TransportRef::McpUrl {
                url: "https://example.invalid/mcp".to_string(),
            },
            credential_ref: Some(CredentialRef::Env {
                var: "FAKE_API_KEY".to_string(),
            }),
        };
        let serialized = serde_json::to_value(&capability).unwrap();
        assert_eq!(serialized["descriptor"]["name"], "fake_tool");
        assert_eq!(serialized["transportRef"]["kind"], "mcpUrl");
        assert_eq!(serialized["credentialRef"]["kind"], "env");
        let parsed: CapabilityRef = serde_json::from_value(serialized).unwrap();
        assert_eq!(parsed, capability);
    }

    #[test]
    fn capability_ref_omits_absent_credential_ref() {
        let capability = CapabilityRef {
            descriptor: sample_descriptor(),
            transport_ref: TransportRef::InProcess {
                component_name: "peer_mcp".to_string(),
            },
            credential_ref: None,
        };
        let serialized = serde_json::to_value(&capability).unwrap();
        assert!(
            serialized.get("credentialRef").is_none(),
            "credentialRef must be omitted when None, got {serialized}",
        );
        let parsed: CapabilityRef = serde_json::from_value(serialized).unwrap();
        assert_eq!(parsed, capability);
    }
}
