//! # Tools Primitive Contract Tests
//!
//! Validates the **Tools** managed-agent primitive against the acceptance bars
//! in `docs/explorations/managed-agents-mapping.md` §6 "Tools" and the
//! Anthropic interface:
//!
//! ```text
//! {name, description, input_schema}
//! ```
//!
//! *"Any capability describable as a name and an input shape — MCP server,
//! custom tool, etc."*
//!
//! The core Tools contract is that every tool the runtime exposes to the agent
//! is fully described by the triple `{name, description, input_schema}` — no
//! transport details, no credentials, no Rust-side type leakage into the
//! capability descriptor the agent sees. Transport and auth resolve at call
//! time via the conductor proxy chain; the schema is the only thing the agent
//! knows about.
//!
//! **Ownership boundary:** portable `credential_ref` + `transport_ref`
//! plumbing (slice 17 Capability profiles) is a Tools *depth* concern that
//! belongs in its own contract test file if and when that slice ships. This
//! file focuses on the schema-only and transport-agnostic invariants.

#[path = "support/managed_agent_suite.rs"]
mod managed_agent_suite;

use anyhow::{Context, Result};
use fireline_harness::{TopologyComponentSpec, TopologySpec};
use managed_agent_suite::{
    DEFAULT_TIMEOUT, LocalRuntimeHarness, ManagedAgentHarnessSpec, create_session,
    wait_for_event_count,
};
use std::collections::HashSet;

/// Precondition: a runtime has been provisioned with a topology that includes
/// `peer_mcp` — the `PeerComponent` registration path that injects
/// `list_peers` and `prompt_peer` as MCP tools per session.
///
/// Action: open an ACP session against the runtime so that the conductor
/// wires the peer MCP server into the proxy chain, then read the durable
/// state stream back and inspect every `tool_descriptor` envelope the
/// topology wire-up emitted.
///
/// Observable evidence: at least one `tool_descriptor` envelope is visible,
/// its `value` contains exactly the Anthropic triple
/// `{name, description, inputSchema}`, and none of the forbidden
/// transport/credential keys (`transport`, `credential`, `host`,
/// `runtimeKey`, `nodeId`) appear anywhere on the value.
///
/// Invariant proven: **Tools schema-only contract** — tool descriptors
/// exposed to the agent contain only the triple Anthropic specifies.
/// Transport details (MCP server URL, peer runtime key, host-tool
/// component name) and credential references (env var names, secret store
/// paths, OAuth scopes) stay in the conductor layer and resolve at call
/// time, never leaking into what the agent can observe or record. The
/// Fireline `tool_descriptor` state envelope is the test-visible mirror
/// of that contract — if it ever drifts off the schema-only shape, this
/// test fails loudly instead of the agent silently learning about
/// transports it shouldn't know about.
#[tokio::test]
async fn tools_schema_only_contract() -> Result<()> {
    let topology = TopologySpec {
        components: vec![TopologyComponentSpec {
            name: "peer_mcp".to_string(),
            config: None,
        }],
    };
    let spec = ManagedAgentHarnessSpec::new("tools-schema-only-contract").with_topology(topology);
    let runtime = LocalRuntimeHarness::spawn_with(spec).await?;

    let result = async {
        // Open a session. The conductor builds its topology on each
        // ACP connection, which triggers the `peer_mcp` factory in
        // src/topology.rs to emit `tool_descriptor` envelopes for
        // every tool the peer MCP server registers. Without this
        // connect step the emission never fires.
        let _session_id = create_session(runtime.acp_url())
            .await
            .context("create session to trigger peer_mcp tool_descriptor emission")?;

        let envelopes =
            wait_for_event_count(runtime.state_stream_url(), "tool_descriptor", 1, DEFAULT_TIMEOUT)
                .await
                .context(
                    "INVARIANT (Tools): at least one tool_descriptor envelope must be visible on \
                     the durable state stream after the conductor wires peer_mcp — no envelopes \
                     means the emission wiring in src/topology.rs did not fire",
                )?;

        let required_keys: HashSet<&str> = ["name", "description", "inputSchema"]
            .into_iter()
            .collect();
        let forbidden_keys: HashSet<&str> =
            ["transport", "credential", "host", "runtimeKey", "nodeId"]
                .into_iter()
                .collect();

        let mut witnessed_names: Vec<String> = Vec::new();

        for envelope in &envelopes {
            assert_eq!(
                envelope.envelope_type(),
                Some("tool_descriptor"),
                "INVARIANT (Tools): filtered envelopes must all be tool_descriptor",
            );
            assert_eq!(
                envelope.operation(),
                Some("insert"),
                "INVARIANT (Tools): tool_descriptor envelopes must use the spec-compliant \
                 `insert` operation (not upsert)",
            );

            let key = envelope.key().unwrap_or_default();
            assert!(
                key.starts_with("peer_mcp:"),
                "INVARIANT (Tools): peer-sourced tool_descriptor key must be prefixed \
                 with `peer_mcp:` for provenance, got `{key}`",
            );

            let value = envelope
                .value()
                .context("tool_descriptor envelope missing value")?;
            let obj = value.as_object().context(
                "INVARIANT (Tools): tool_descriptor value must be a JSON object, not a scalar \
                 or array",
            )?;

            let present_keys: HashSet<&str> = obj.keys().map(String::as_str).collect();
            assert_eq!(
                present_keys, required_keys,
                "INVARIANT (Tools): tool_descriptor value must carry exactly the Anthropic \
                 triple {{name, description, inputSchema}}, got keys: {present_keys:?}",
            );

            for forbidden in &forbidden_keys {
                assert!(
                    !present_keys.contains(forbidden),
                    "INVARIANT (Tools): tool_descriptor value must not leak `{forbidden}`; \
                     transport and credential resolution stay in the conductor layer",
                );
            }

            let name = value
                .get("name")
                .and_then(|v| v.as_str())
                .context("tool_descriptor.value.name must be a string")?;
            assert!(
                !name.is_empty(),
                "INVARIANT (Tools): tool_descriptor.value.name must be non-empty",
            );

            let description = value
                .get("description")
                .and_then(|v| v.as_str())
                .context("tool_descriptor.value.description must be a string")?;
            assert!(
                !description.is_empty(),
                "INVARIANT (Tools): tool_descriptor.value.description must be non-empty",
            );

            let input_schema = value
                .get("inputSchema")
                .context("tool_descriptor.value.inputSchema must be present")?;
            assert!(
                input_schema.is_object() || input_schema.is_boolean(),
                "INVARIANT (Tools): tool_descriptor.value.inputSchema must be a JSON Schema \
                 document (object or boolean), got {input_schema}",
            );

            witnessed_names.push(name.to_string());
        }

        // The peer MCP server registers exactly two tools today. If
        // the set ever grows/shrinks, update the assertion — but we
        // assert against the set rather than the count to keep the
        // failure message descriptive.
        let witnessed: HashSet<&str> = witnessed_names.iter().map(String::as_str).collect();
        let expected_peer_tools: HashSet<&str> = ["list_peers", "prompt_peer"].into_iter().collect();
        assert!(
            expected_peer_tools.is_subset(&witnessed),
            "INVARIANT (Tools): peer_mcp must emit tool_descriptor envelopes for every tool \
             it registers with the MCP server. Expected at least {expected_peer_tools:?}, \
             witnessed {witnessed:?}",
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: two tools are registered through the new `attach_tool`
/// topology component, each carrying a **different** `TransportRef`
/// variant but still exposing the Anthropic-shape `{name, description,
/// input_schema}` descriptor.
///
/// Action: open an ACP session so the conductor wires `attach_tool`
/// into the proxy chain, triggering `tool_descriptor` emission for
/// every capability in the list.
///
/// Observable evidence: exactly two `tool_descriptor` envelopes land on
/// the durable state stream, one per capability; each envelope's
/// `value` contains exactly the Anthropic triple
/// `{name, description, inputSchema}` with no transport or credential
/// keys leaked through the wire.
///
/// Invariant proven: **Tools transport-agnostic registration** — the
/// agent cannot distinguish a tool attached through one transport from
/// the same-shaped tool attached through another. The wire value is
/// always the `ToolDescriptor` triple; `transport_ref` and
/// `credential_ref` stay inside the conductor layer. Slice 17's
/// `CapabilityRef` shape (descriptor + transport_ref + credential_ref)
/// makes this invariant cheap to test: the same `attach_tool`
/// component handles both `TransportRef::InProcess` and
/// `TransportRef::McpUrl` entries with zero difference visible on the
/// wire.
///
/// # Slice 17 scope boundary
/// This test covers the descriptor-emission surface only. Live tool
/// dispatch through `TransportRef` (connecting to the remote MCP URL,
/// forwarding the call, resolving credentials) is an explicit
/// follow-up slice — there is no assertion here that the two tools are
/// callable, only that their descriptors project onto the stream with
/// the correct shape.
#[tokio::test]
async fn tools_transport_agnostic_registration() -> Result<()> {
    let topology = TopologySpec {
        components: vec![TopologyComponentSpec {
            name: "attach_tool".to_string(),
            config: Some(serde_json::json!({
                "capabilities": [
                    {
                        "descriptor": {
                            "name": "inproc_tool",
                            "description": "Tool fetched via in-process component delegation.",
                            "inputSchema": {"type": "object"}
                        },
                        "transportRef": {
                            "kind": "inProcess",
                            "componentName": "peer_mcp"
                        }
                    },
                    {
                        "descriptor": {
                            "name": "remote_tool",
                            "description": "Tool fetched via remote MCP URL.",
                            "inputSchema": {"type": "object"}
                        },
                        "transportRef": {
                            "kind": "mcpUrl",
                            "url": "https://example.invalid/fake-mcp"
                        },
                        "credentialRef": {
                            "kind": "env",
                            "var": "FAKE_API_KEY"
                        }
                    }
                ]
            })),
        }],
    };
    let spec = ManagedAgentHarnessSpec::new("tools-transport-agnostic-registration")
        .with_topology(topology);
    let runtime = LocalRuntimeHarness::spawn_with(spec).await?;

    let result = async {
        // Open a session so the conductor builds its topology and the
        // attach_tool component runs its emission loop.
        let _session_id = create_session(runtime.acp_url())
            .await
            .context("create session to trigger attach_tool emission")?;

        let envelopes =
            wait_for_event_count(runtime.state_stream_url(), "tool_descriptor", 2, DEFAULT_TIMEOUT)
                .await
                .context(
                    "INVARIANT (Tools): attach_tool must emit one tool_descriptor envelope \
                     per CapabilityRef on conductor wire-up, regardless of the capability's \
                     transport variant",
                )?;

        let required_keys: HashSet<&str> =
            ["name", "description", "inputSchema"].into_iter().collect();
        let forbidden_keys: HashSet<&str> = [
            "transport",
            "transportRef",
            "credential",
            "credentialRef",
            "host",
            "runtimeKey",
            "nodeId",
        ]
        .into_iter()
        .collect();

        let mut witnessed_names: HashSet<String> = HashSet::new();

        for envelope in &envelopes {
            assert_eq!(
                envelope.envelope_type(),
                Some("tool_descriptor"),
                "INVARIANT (Tools): filtered envelopes must all be tool_descriptor",
            );
            assert_eq!(
                envelope.operation(),
                Some("insert"),
                "INVARIANT (Tools): tool_descriptor envelopes must use `insert`",
            );

            let key = envelope.key().unwrap_or_default();
            assert!(
                key.starts_with("attach_tool:"),
                "INVARIANT (Tools): attach_tool-sourced tool_descriptor keys must be prefixed \
                 with `attach_tool:` for provenance, got `{key}`",
            );

            let value = envelope
                .value()
                .context("tool_descriptor envelope missing value")?;
            let obj = value.as_object().context(
                "INVARIANT (Tools): tool_descriptor value must be a JSON object",
            )?;
            let present_keys: HashSet<&str> = obj.keys().map(String::as_str).collect();
            assert_eq!(
                present_keys, required_keys,
                "INVARIANT (Tools): tool_descriptor value must carry exactly the Anthropic \
                 triple {{name, description, inputSchema}} regardless of transport. Got \
                 keys: {present_keys:?}. The transport_ref / credential_ref half of \
                 CapabilityRef must NEVER leak onto the wire.",
            );
            for forbidden in &forbidden_keys {
                assert!(
                    !present_keys.contains(forbidden),
                    "INVARIANT (Tools): tool_descriptor value must not leak `{forbidden}`; \
                     the wire value is the Anthropic triple, not the CapabilityRef shape",
                );
            }

            let name = value
                .get("name")
                .and_then(|v| v.as_str())
                .context("tool_descriptor.value.name must be a string")?;
            witnessed_names.insert(name.to_string());
        }

        let expected: HashSet<&str> = ["inproc_tool", "remote_tool"].into_iter().collect();
        let witnessed_refs: HashSet<&str> = witnessed_names.iter().map(String::as_str).collect();
        assert_eq!(
            witnessed_refs, expected,
            "INVARIANT (Tools): attach_tool must emit descriptors for every configured \
             capability, regardless of which TransportRef variant carries them. \
             Expected {expected:?}, witnessed {witnessed_refs:?}.",
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: one `attach_tool` topology component configured with
/// two `CapabilityRef` values that share a `descriptor.name` but
/// differ in `description`.
///
/// Action: open an ACP session so the `attach_tool` emission loop runs
/// against the two colliding capabilities.
///
/// Observable evidence: exactly **one** `tool_descriptor` envelope for
/// the shared name lands on the state stream, and its description
/// matches the **first** capability's description. The second
/// capability's emission is suppressed by the first-attach-wins rule
/// (the loser's descriptor never lands on the wire).
///
/// Invariant proven: **First-attach-wins collision rule** — the
/// `attach_tool` component is the single source of truth for
/// resolving same-name collisions between `CapabilityRef` entries.
/// The rule is deterministic (list order) and documented in the
/// `attach_tool` module docstring. Without this, a topology carrying
/// the same logical tool through two transports would project two
/// conflicting descriptors, and the agent would see a non-deterministic
/// tool surface at session wire-up.
#[tokio::test]
async fn tools_first_attach_wins_on_name_collision() -> Result<()> {
    let topology = TopologySpec {
        components: vec![TopologyComponentSpec {
            name: "attach_tool".to_string(),
            config: Some(serde_json::json!({
                "capabilities": [
                    {
                        "descriptor": {
                            "name": "shared_tool",
                            "description": "first attach — should win",
                            "inputSchema": {"type": "object"}
                        },
                        "transportRef": {
                            "kind": "inProcess",
                            "componentName": "peer_mcp"
                        }
                    },
                    {
                        "descriptor": {
                            "name": "shared_tool",
                            "description": "second attach — should be suppressed",
                            "inputSchema": {"type": "object"}
                        },
                        "transportRef": {
                            "kind": "mcpUrl",
                            "url": "https://example.invalid/second"
                        }
                    }
                ]
            })),
        }],
    };
    let spec = ManagedAgentHarnessSpec::new("tools-first-attach-wins-collision")
        .with_topology(topology);
    let runtime = LocalRuntimeHarness::spawn_with(spec).await?;

    let result = async {
        let _session_id = create_session(runtime.acp_url())
            .await
            .context("create session to trigger attach_tool collision emission")?;

        // Wait for at least one envelope, then assert we have *exactly*
        // one by counting all tool_descriptor envelopes that carry
        // name="shared_tool".
        let _ = wait_for_event_count(
            runtime.state_stream_url(),
            "tool_descriptor",
            1,
            DEFAULT_TIMEOUT,
        )
        .await
        .context(
            "INVARIANT (Tools): attach_tool must emit at least one tool_descriptor \
             envelope even when the capability list contains a name collision",
        )?;

        // Now read every tool_descriptor envelope on the stream and
        // filter to the shared name to confirm first-attach-wins.
        let envelopes = wait_for_event_count(
            runtime.state_stream_url(),
            "tool_descriptor",
            1,
            DEFAULT_TIMEOUT,
        )
        .await?;
        let shared: Vec<_> = envelopes
            .iter()
            .filter(|env| {
                env.value()
                    .and_then(|v| v.get("name"))
                    .and_then(|v| v.as_str())
                    == Some("shared_tool")
            })
            .collect();

        assert_eq!(
            shared.len(),
            1,
            "INVARIANT (Tools): first-attach-wins must collapse same-name capabilities to a \
             single tool_descriptor envelope on the wire. Observed {} envelopes for \
             name=shared_tool.",
            shared.len()
        );

        let winner = shared[0];
        let description = winner
            .value()
            .and_then(|v| v.get("description"))
            .and_then(|v| v.as_str())
            .context("winner tool_descriptor.value.description must be a string")?;
        assert_eq!(
            description, "first attach — should win",
            "INVARIANT (Tools): first-attach-wins means the FIRST capability in the \
             list is the one whose descriptor lands on the stream. Got description: \
             `{description}`",
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}
