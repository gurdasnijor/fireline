//! Standalone managed-agent acceptance suite.
//!
//! This file is additive. It does not replace or mutate the existing
//! `managed_agent_primitives_e2e.rs` executable spec. The goal here is a
//! higher-signal, easier-to-evolve suite shape:
//!
//! - one shared support module for runtime/control-plane bring-up
//! - small primitive-oriented tests instead of one giant lifecycle narrative
//! - explicit tracking of which contracts are Rust-owned versus TypeScript-owned
//!
//! The Rust suite should validate substrate truths. TypeScript-owned adapter
//! contracts still need package-local tests in `packages/client` and
//! `packages/state`.

#[path = "support/managed_agent_suite.rs"]
mod managed_agent_suite;

use anyhow::Result;
use managed_agent_suite::{
    contract_inventory, covered_primitives, pending_contract, LocalRuntimeHarness, Primitive,
    SurfaceOwner, DEFAULT_TIMEOUT,
};

#[test]
fn managed_agent_contract_inventory_explicitly_spans_rust_and_typescript() {
    let inventory = contract_inventory();

    assert_eq!(
        covered_primitives().len(),
        6,
        "inventory should represent all six Anthropic primitives"
    );
    assert!(
        inventory
            .iter()
            .any(|case| case.owner == SurfaceOwner::RustSubstrate),
        "inventory should keep Rust-owned substrate contracts explicit"
    );
    assert!(
        inventory
            .iter()
            .any(|case| case.owner == SurfaceOwner::TypeScriptState),
        "inventory should track packages/state-owned contracts explicitly"
    );
    assert!(
        inventory
            .iter()
            .any(|case| case.owner == SurfaceOwner::TypeScriptClient),
        "inventory should track packages/client-owned contracts explicitly"
    );
    assert!(
        inventory.iter().any(|case| {
            case.id == "session.external_consumer"
                && case.primitive == Primitive::Session
                && case.owner == SurfaceOwner::TypeScriptState
        }),
        "Session external-consumer proof belongs in packages/state"
    );
    assert!(
        inventory.iter().any(|case| {
            case.id == "orchestration.resume_helper"
                && case.primitive == Primitive::Orchestration
                && case.owner == SurfaceOwner::TypeScriptClient
        }),
        "resume(sessionId) should be tracked as a packages/client-owned invariant"
    );
    assert!(
        inventory.iter().any(|case| {
            case.id == "resources.launch_spec"
                && case.primitive == Primitive::Resources
                && case.owner == SurfaceOwner::CrossSurface
        }),
        "resources launch-spec work should stay marked as a cross-surface contract"
    );
}

#[tokio::test]
async fn managed_agent_baseline_smoke_validates_session_harness_and_sandbox() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("managed-agent-suite-smoke").await?;

    let result = async {
        let response = runtime
            .prompt("hello from the managed-agent standalone suite")
            .await?;
        assert!(
            response.contains("Hello"),
            "test agent should answer the prompt through ACP: {response}"
        );

        let body = runtime
            .wait_for_state_rows(
                &[
                    "\"type\":\"connection\"",
                    "\"type\":\"session\"",
                    "\"type\":\"prompt_turn\"",
                    "\"type\":\"chunk\"",
                ],
                DEFAULT_TIMEOUT,
            )
            .await?;

        assert!(
            body.contains("\"state\":\"completed\""),
            "prompt turns should eventually materialize as completed rows: {body}"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

#[tokio::test]
#[ignore = "pending slice 14 + slice 16 + packages/client resume(sessionId)"]
async fn managed_agent_orchestration_acceptance_contract() -> Result<()> {
    pending_contract(
        "orchestration.resume",
        "Split this into: durable runtimeSpec persistence, a resumer subscriber loop, and a packages/client end-to-end resume(sessionId) test once the TS API lands.",
    )
}

#[tokio::test]
#[ignore = "pending slice 15 physical ResourceMounter work"]
async fn managed_agent_resources_physical_mount_acceptance_contract() -> Result<()> {
    pending_contract(
        "resources.physical_mounts",
        "Add a control-plane-backed runtime test that passes resources in the launch spec, mounts them before first prompt, and proves shell-visible reads inside the runtime.",
    )
}

#[tokio::test]
#[ignore = "pending slice 15 FsBackendComponent + artifact materialization"]
async fn managed_agent_resources_fs_backend_acceptance_contract() -> Result<()> {
    pending_contract(
        "resources.fs_backend",
        "Cover ACP fs interception separately from physical mounts: write via fs/write_text_file, assert fs_op durable evidence, then prove cross-runtime reads or projection-backed artifact lookup.",
    )
}

#[tokio::test]
#[ignore = "pending richer init-effect inspection for schema-only tool registration"]
async fn managed_agent_tools_schema_only_acceptance_contract() -> Result<()> {
    pending_contract(
        "tools.schema_only",
        "Once the init effect is easy to inspect in Rust, assert that tool registration exposes {name, description, input_schema} without transport details or credentials.",
    )
}
