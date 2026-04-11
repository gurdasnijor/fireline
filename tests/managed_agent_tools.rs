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

use anyhow::Result;
use managed_agent_suite::pending_contract;

/// Precondition: a runtime has been provisioned with a topology that includes
/// `PeerComponent` (which registers `list_peers` and `prompt_peer` as MCP
/// tools) and one tool registration from an external source (e.g.
/// `SmitheryComponent`).
///
/// Action: open an ACP session against the runtime, issue an `initialize`
/// request, and inspect the `init` effect's tool registration list.
///
/// Observable evidence: every tool in the init list carries only the fields
/// `{name, description, input_schema}` — no `transport_ref`, no
/// `credential_ref`, no internal component identifiers, no host paths, no
/// file descriptors.
///
/// Invariant proven: **Tools schema-only contract** — tool descriptors
/// exposed to the agent contain only the triple Anthropic specifies.
/// Transport details (MCP server URL, peer runtime key, host-tool component
/// name) and credential references (env var names, secret store paths, OAuth
/// scopes) stay in the conductor layer and resolve at call time, never
/// leaking into what the agent can observe or record.
#[tokio::test]
#[ignore = "pending: needs a test-side helper to inspect the ACP init effect's tool list \
            without reimplementing the conductor's init handling; promote once the \
            shared harness exposes an init-inspection method or a dedicated tool-registry \
            query path"]
async fn tools_schema_only_contract() -> Result<()> {
    pending_contract(
        "tools.schema_only",
        "Extend tests/support/managed_agent_suite.rs with a helper that opens an ACP \
         session against a LocalRuntimeHarness, fetches the init effect's tool list, \
         and returns it as a parsed structure. Then assert each entry has exactly the \
         {name, description, input_schema} keys (optionally with an MCP-standard \
         identifier field) and none of the transport/credential/host-internal keys. \
         Requires either a new harness method or an inline ACP client setup.",
    )
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
#[ignore = "pending: slice 17 Capability profiles + a documented collision resolution rule"]
async fn tools_transport_agnostic_registration() -> Result<()> {
    pending_contract(
        "tools.transport_agnostic",
        "Pin the collision resolution rule in the conductor first (probably \
         'first registration wins' or 'last wins' — whichever is actually documented). \
         Then register the same tool name via two different transports and assert the \
         agent sees exactly one registration with the intended transport resolved at \
         call time. Blocks on slice 17 work and on whichever resolution rule the \
         conductor actually enforces.",
    )
}
