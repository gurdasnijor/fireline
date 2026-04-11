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
#[ignore = "pending: Fireline does not model an inspectable 'init effect tool list' today. \
            The ACP wire protocol has no available_tools field on InitializeResponse or \
            NewSessionResponse — the SDK schema crate carries no such field. Tools live \
            inside MCP servers attached via NewSessionRequest.mcp_servers and are only \
            discoverable via an MCP tools/list call by the agent. The conductor's \
            state_projector reads the initialize response solely for \
            agentCapabilities.loadSession and emits no tool_descriptor envelope. The \
            McpServer SDK type stores tools in a private HashMap with no public Rust \
            accessor — only the rmcp ServerHandler::list_tools wire path. Promoting this \
            test requires either (a) a small SDK addition exposing McpServer::tools() so \
            tests can introspect the registered Tool triple directly, or (b) a Fireline \
            production addition that mirrors registered MCP tool descriptors onto the \
            durable state stream as `tool_descriptor` envelopes (then read via \
            read_all_events). Until one of those exists, the Tools row in \
            docs/explorations/managed-agents-mapping.md §6 is overstated: \
            `crates/fireline-components/src/tools.rs` referenced by that doc DOES NOT \
            EXIST. The only Fireline component code is PeerComponent and \
            SmitheryComponent, both of which build their MCP servers via the SDK \
            mcp_server::McpServer::builder().tool_fn(...) chain and are introspectable \
            only over the MCP wire."]
async fn tools_schema_only_contract() -> Result<()> {
    pending_contract(
        "tools.schema_only",
        "Blocked on production substrate, not test wiring. The schema-only contract \
         requires an observable that lets a test see the `{name, description, input_schema}` \
         triple a registered tool exposes to the agent. Today: \
         (1) ACP InitializeResponse carries no tools field (verified in \
         agent-client-protocol-core/src/schema), \
         (2) the conductor state_projector emits no tool_descriptor envelope, \
         (3) the SDK's McpServer keeps registered tools in a private HashMap with no \
         public Rust accessor, \
         (4) the only existing live probe — TestyCommand::ListTools driving an MCP \
         tools/list against the attached server (see tests/mesh_baseline.rs:118) — \
         renders `name: description` strings and discards input_schema, so it cannot \
         witness the third leg of the triple. \
         Minimum production change: emit a `tool_descriptor` envelope on the durable \
         state stream when the conductor wires an MCP server into a session, carrying \
         the `{name, description, input_schema}` triple per registered tool. The test \
         can then read these via read_all_events and assert the keys are exactly the \
         triple. Alternative: upstream a small `McpServer::tool_definitions()` \
         accessor in the rust-sdk that returns the registered rmcp Tool list, then \
         expose it through an in-process harness handle. Either path is one focused \
         change; the fixture/oracle infrastructure (LocalRuntimeHarness, \
         ManagedAgentHarnessSpec, parsed envelope reader) is already in place.",
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
#[ignore = "pending: same-name collision across transports cannot even be constructed in \
            Fireline today. Each topology component injects its own MCP server with \
            distinct top-level tool names: `peer_mcp` exposes {list_peers, prompt_peer}; \
            SmitheryComponent exposes {smithery_call}. There is no `attachTool` API in \
            either the Rust crates or the SDK — tools are always registered as members \
            of an MCP server via sacp::mcp_server::McpServer::builder().tool_fn(...) and \
            the only way to expose 'the same tool' through two transports would be to \
            stand up two MCP servers with overlapping tool names AND have something \
            (the conductor or the agent) collapse them. Neither exists. The conductor \
            does not flatten tool names across MCP servers; the agent reaches each \
            server separately via the standard MCP server-name-scoped routing. There is \
            also no documented collision resolution rule. Slice 17 (capability profiles \
            with portable transport_ref / credential_ref) is the production substrate \
            this test was written against, and it has not started."]
async fn tools_transport_agnostic_registration() -> Result<()> {
    pending_contract(
        "tools.transport_agnostic",
        "Blocked on a production primitive that does not yet exist. \
         To even set up the precondition we need: \
         (1) a registration path that can mount the same logical tool through more than \
         one transport (slice 17 Capability profiles — not started), \
         (2) a documented collision resolution rule in the conductor (no rule exists; \
         tools are scoped per MCP server today, not flattened into one namespace), \
         (3) the same observable infrastructure required by tools_schema_only_contract \
         — see that test's pending_contract message. \
         Until slice 17 ships an `attachTool`-shaped API plus a defined collision rule, \
         this test has no fixture to write. Recommendation: leave ignored, do not draft \
         a placeholder. When slice 17 lands, the same tool_descriptor stream envelope \
         (or equivalent McpServer accessor) proposed for tools_schema_only_contract \
         will also unblock this one — the assertion becomes 'after registering tool X \
         via two transports, exactly one tool_descriptor with name=X is visible and \
         its routing target matches the documented rule'.",
    )
}
