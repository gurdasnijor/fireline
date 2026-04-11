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

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::{
    ReadTextFileRequest, ReadTextFileResponse, WriteTextFileRequest, WriteTextFileResponse,
};
use anyhow::Result;
use durable_streams::Client as DsClient;
use fireline_components::fs_backend::{FsBackendComponent, LocalFileBackend};
use fireline_conductor::runtime::{LocalPathMounter, ResourceMounter, ResourceRef};
use managed_agent_suite::{
    ControlPlaneHarness, create_session, load_session, prompt_session, testy_load_bin,
};
use managed_agent_suite::{
    DEFAULT_TIMEOUT, LocalRuntimeHarness, Primitive, SurfaceOwner, contract_inventory,
    covered_primitives, pending_contract, temp_path,
};
use uuid::Uuid;

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
async fn managed_agent_orchestration_acceptance_contract() -> Result<()> {
    let control_plane = ControlPlaneHarness::spawn(true).await?;

    let result = async {
        let runtime = control_plane
            .create_runtime_with_agent(
                "managed-agent-orchestration",
                &[testy_load_bin().display().to_string()],
            )
            .await?;

        let persisted = fireline::orchestration::reconstruct_runtime_spec_from_log(
            &runtime.state.url,
            &runtime.runtime_key,
        )
        .await?;
        assert_eq!(persisted.runtime_key, runtime.runtime_key);
        assert_eq!(
            persisted.create_spec.runtime_key.as_deref(),
            Some(runtime.runtime_key.as_str())
        );
        assert_eq!(
            persisted.create_spec.node_id.as_deref(),
            Some(runtime.node_id.as_str())
        );

        let session_id = create_session(&runtime.acp.url).await?;
        let _ = prompt_session(
            &runtime.acp.url,
            &session_id,
            "hello before orchestrated resume",
        )
        .await?;

        let _stopped = control_plane.stop_runtime(&runtime.runtime_key).await?;
        let resumed = fireline::orchestration::resume(
            &control_plane.http,
            &control_plane.base_url,
            &session_id,
        )
        .await?;

        assert_eq!(resumed.runtime_key, runtime.runtime_key);
        assert_eq!(resumed.status, fireline::runtime_host::RuntimeStatus::Ready);
        assert_ne!(
            resumed.runtime_id, runtime.runtime_id,
            "cold-start resume should produce a new runtime process identity"
        );

        load_session(&resumed.acp.url, &session_id).await?;
        let _ = prompt_session(
            &resumed.acp.url,
            &session_id,
            "hello after orchestrated resume",
        )
        .await?;

        Ok(())
    }
    .await;

    control_plane.shutdown().await;
    result
}

#[tokio::test]
async fn managed_agent_resources_physical_mount_acceptance_contract() -> Result<()> {
    let source_dir = temp_path("managed-agent-physical-mount");
    std::fs::create_dir_all(&source_dir)?;
    std::fs::write(source_dir.join("hello.txt"), "hello from resources")?;

    let result = async {
        let mounter = LocalPathMounter::new();
        let mounted = mounter
            .mount(
                &ResourceRef::LocalPath {
                    path: source_dir.clone(),
                    mount_path: PathBuf::from("/workspace"),
                },
                "runtime:managed-agent-physical-mount",
            )
            .await?
            .expect("local path resource should mount");

        assert_eq!(mounted.host_path, std::fs::canonicalize(&source_dir)?);
        assert_eq!(mounted.mount_path, PathBuf::from("/workspace"));
        assert!(
            mounted.read_only,
            "local path mounts should be read-only by default"
        );

        let backend = LocalFileBackend::new(vec![mounted]);
        let bytes = fireline_components::fs_backend::FileBackend::read(
            &backend,
            PathBuf::from("/workspace/hello.txt").as_path(),
        )
        .await?;
        assert_eq!(String::from_utf8(bytes)?, "hello from resources");

        Ok(())
    }
    .await;

    let _ = std::fs::remove_dir_all(&source_dir);
    result
}

#[tokio::test]
#[ignore = "pending shell-visible resource read end-to-end through a launched runtime"]
async fn managed_agent_resources_physical_mount_shell_visibility_contract() -> Result<()> {
    pending_contract(
        "resources.physical_mounts.shell_visible_read",
        "Blocked on an end-to-end runtime/agent path that proves shell-visible reads inside the launched runtime. ResourceMounter coverage is handled by managed_agent_resources_physical_mount_acceptance_contract.",
    )
}

#[tokio::test]
#[ignore = "pending prompt-driven ACP fs emission from a test agent"]
async fn managed_agent_resources_fs_backend_acceptance_contract() -> Result<()> {
    pending_contract(
        "resources.fs_backend",
        "Blocked on testy fs-op emission, covered at component layer via managed_agent_resources_fs_backend_component_test.",
    )
}

#[tokio::test]
async fn managed_agent_resources_fs_backend_component_test() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("managed-agent-fs-backend-component").await?;
    let scratch_dir = temp_path("managed-agent-fs-backend");
    std::fs::create_dir_all(&scratch_dir)?;
    let artifact_path = scratch_dir.join("artifact.txt");
    let artifact_text = "artifact written through fs backend";
    let session_id = format!("session:{}", Uuid::new_v4());

    let result = async {
        let component = FsBackendComponent::new(
            Arc::new(LocalFileBackend::new(Vec::new())),
            state_stream_producer(runtime.state_stream_url()),
        );

        let write = component
            .handle_write_text_file(&WriteTextFileRequest::new(
                session_id.clone(),
                &artifact_path,
                artifact_text,
            ))
            .await?;
        let WriteTextFileResponse { .. } = write;

        assert_eq!(std::fs::read_to_string(&artifact_path)?, artifact_text);

        let read = component
            .handle_read_text_file(&ReadTextFileRequest::new(
                session_id.clone(),
                &artifact_path,
            ))
            .await?;
        let ReadTextFileResponse { content, .. } = read;
        assert_eq!(content, artifact_text);

        let body = runtime
            .wait_for_state_rows(
                &[
                    "\"type\":\"fs_op\"",
                    "\"op\":\"write\"",
                    &session_id,
                    artifact_path.to_string_lossy().as_ref(),
                    artifact_text,
                ],
                DEFAULT_TIMEOUT,
            )
            .await?;
        assert!(
            body.contains("\"type\":\"fs_op\""),
            "fs writes should append durable fs_op evidence: {body}"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    let _ = std::fs::remove_dir_all(&scratch_dir);
    result
}

#[tokio::test]
#[ignore = "pending richer init-effect inspection for schema-only tool registration"]
async fn managed_agent_tools_schema_only_acceptance_contract() -> Result<()> {
    pending_contract(
        "tools.schema_only",
        "Once the init effect is easy to inspect in Rust, assert that tool registration exposes {name, description, input_schema} without transport details or credentials.",
    )
}

fn state_stream_producer(state_stream_url: &str) -> durable_streams::Producer {
    let client = DsClient::new();
    let mut stream = client.stream(state_stream_url);
    stream.set_content_type("application/json");
    stream
        .producer(format!("managed-agent-suite-{}", Uuid::new_v4()))
        .content_type("application/json")
        .build()
}
