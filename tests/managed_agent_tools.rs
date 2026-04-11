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
use fireline_conductor::topology::{TopologyComponentSpec, TopologySpec};
use managed_agent_suite::{
    DEFAULT_TIMEOUT, LocalRuntimeHarness, ManagedAgentHarnessSpec, create_session,
    pending_contract, wait_for_event_count,
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

/// Precondition: two tools are registered with the same `name` but through
/// different transports (one via `PeerComponent`, one via `SmitheryComponent`,
/// one via a custom `attachTool`).
///
/// Action: observe how the conductor's init effect surfaces these to the
/// agent — does it enforce name uniqueness, does it window by source, does
/// it last-wins, does it reject the second registration?
///
/// Observable evidence: the init effect's tool list has a deterministic
/// resolution rule for same-name collisions.
///
/// Invariant proven: **Tools transport-agnostic registration** — the agent
/// cannot distinguish a tool registered via one transport from the same tool
/// registered via another. The conductor collapses them to a single schema
/// and resolves the transport at call time, which is the contract that makes
/// tool portability (slice 17 Capability profiles) possible.
#[tokio::test]
#[ignore = "pending: schema-only observability is unblocked (see \
            tools_schema_only_contract, which promotes a real test against the \
            `tool_descriptor` state envelope emitted by src/topology.rs at peer_mcp \
            wire-up time). The missing piece is the *collision-resolution* half of \
            the contract: same-name tools registered through more than one transport \
            cannot yet be constructed in Fireline because there is no attachTool- \
            shaped registration API carrying portable transport_ref / credential_ref \
            handles, and no conductor rule for resolving collisions between tools \
            sourced from different MCP servers. Today each topology component scopes \
            its tool namespace (peer_mcp: {list_peers, prompt_peer}; \
            SmitheryComponent: {smithery_call}) so a collision can't even be wired. \
            Slice 17 (capability profiles with portable refs + first-attach-wins \
            collision rule) is the production substrate this test was written \
            against, and it has not started. When slice 17 lands, the assertion \
            becomes: 'after registering tool X via two transports, exactly one \
            tool_descriptor envelope with name=X is visible, its key prefix records \
            which transport won the collision, and the value still carries only the \
            Anthropic triple {name, description, inputSchema}'."]
async fn tools_transport_agnostic_registration() -> Result<()> {
    pending_contract(
        "tools.transport_agnostic",
        "Blocked on slice 17 (Capability profiles). The schema-only observability \
         half of the Tools contract is now live — tools_schema_only_contract \
         witnesses the `tool_descriptor` state envelope emitted at peer_mcp wire-up. \
         What this test still needs: \
         (1) an attachTool-shaped registration path that accepts portable \
         `transport_ref` and `credential_ref` handles rather than an embedded \
         transport (not in Fireline or the SDK yet), \
         (2) a documented collision-resolution rule in the conductor — specifically \
         first-attach-wins across different transports, with the loser's \
         tool_descriptor either suppressed or annotated for provenance, \
         (3) a fixture that can mount the same logical tool through two transports \
         simultaneously so the collision actually occurs. \
         None of (1)-(3) exist today; tools are MCP-server-scoped per topology \
         component, so the collision can't even be constructed. When slice 17 \
         ships, the assertion becomes: register tool X via two transports, observe \
         that exactly one tool_descriptor envelope with name=X lands on the stream, \
         its key prefix identifies the winning transport, and its value keys are \
         exactly {name, description, inputSchema}.",
    )
}
