//! # Resources Primitive Contract Tests
//!
//! Validates the **Resources** managed-agent primitive against the acceptance
//! bars in `docs/explorations/managed-agents-mapping.md` §5 "Resources" and the
//! Anthropic interface:
//!
//! ```text
//! [{source_ref, mount_path}]
//! ```
//!
//! *"Any object store the container can fetch from by reference — Filestore,
//! GCS, a git remote, S3."*
//!
//! Resources splits into two halves per the reduction in the mapping doc §5:
//!
//! 1. **Physical mounts** for shell-based agents (slice 15 `ResourceMounter` +
//!    `LocalPathMounter` + `GitRemoteMounter` — committed as `a6a74ec`)
//! 2. **ACP fs interception** via `FsBackendComponent` for ACP-native file ops
//!    (also in `a6a74ec`; composable via `compose(substitute, appendToSession)`)
//!
//! This file tests both halves as separable contracts. The ownership boundary:
//! the component-layer contracts (LocalPathMounter returns valid
//! `MountedResource`, LocalFileBackend round-trips reads through mounts) run
//! here; the **end-to-end agent-driven** contracts (a shell inside the runtime
//! can actually `cat /work/hello.txt`, an agent can actually emit
//! `fs/write_text_file`) are pending on scripted-testy work.

#[path = "support/managed_agent_suite.rs"]
mod managed_agent_suite;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use fireline_harness::{TopologyComponentSpec, TopologySpec};
use fireline_resources::{
    FileBackend, LocalFileBackend, LocalPathMounter, MountedResource, ResourceMounter, ResourceRef,
    ResourceSourceRef, StreamFsFileBackend,
};
use managed_agent_suite::{
    DEFAULT_TIMEOUT, LocalRuntimeHarness, ManagedAgentHarnessSpec, create_session,
    pending_contract, prompt_session, temp_path, testy_fs_bin, wait_for_event_count,
};

/// Precondition: a source directory on the host filesystem containing a
/// known file (`hello.txt` → "hello from resources").
///
/// Action: construct a `ResourceRef::LocalPath` pointing at that directory
/// with `mount_path: /workspace`. Invoke `LocalPathMounter::mount` against
/// it, receiving a `MountedResource` back.
///
/// Observable evidence: the returned `MountedResource.host_path` is the
/// canonicalized source directory, and `mount_path` is `/workspace`. No
/// other mounter invocations or ambient state are required.
///
/// Invariant proven: **Resources physical mount mapping** — the slice 15
/// `LocalPathMounter` faithfully translates a `source_ref → mount_path` pair
/// into a `MountedResource` that the runtime provider can use to set up the
/// actual container mount. Prior to shell-visible end-to-end tests, this is
/// the component-layer proof that the mounter trait implementation is correct.
#[tokio::test]
async fn resources_local_path_mounter_maps_source_to_mount() -> Result<()> {
    let source_dir = temp_path("resources-local-path-mount");
    std::fs::create_dir_all(&source_dir)?;
    std::fs::write(source_dir.join("hello.txt"), "hello from resources")?;

    let mounter = LocalPathMounter::new();
    let mounted = mounter
        .mount(
            &ResourceRef {
                source_ref: ResourceSourceRef::LocalPath {
                    host_id: String::new(),
                    path: source_dir.clone(),
                },
                mount_path: PathBuf::from("/workspace"),
                read_only: true,
            },
            "runtime:resources-local-path",
        )
        .await
        .context("LocalPathMounter::mount should succeed for an existing local dir")?
        .expect("LocalPathMounter should return Some(mounted) for a LocalPath ref");

    assert_eq!(
        mounted.host_path,
        std::fs::canonicalize(&source_dir)?,
        "INVARIANT (Resources): host_path is the canonicalized source"
    );
    assert_eq!(
        mounted.mount_path,
        PathBuf::from("/workspace"),
        "INVARIANT (Resources): mount_path matches the requested mount"
    );

    let _ = std::fs::remove_dir_all(&source_dir);
    Ok(())
}

/// Precondition: a `LocalFileBackend` has been constructed with a single
/// `MountedResource` pointing at a seeded source directory.
///
/// Action: call `backend.read(Path::new("/workspace/hello.txt"))` — the
/// backend must translate the mount path to the host path and return the
/// file contents.
///
/// Observable evidence: the returned bytes match the seeded file contents.
///
/// Invariant proven: **Resources FsBackendComponent route-through** — the
/// slice 15 `LocalFileBackend` correctly uses `MountedResource` mapping to
/// translate ACP-style mount paths into host reads. This is one half of the
/// composable ACP fs interception that the mapping doc §5 calls out as the
/// `compose(substitute, appendToSession)` combinator pattern.
#[tokio::test]
async fn resources_local_file_backend_reads_through_mount_mapping() -> Result<()> {
    let source_dir = temp_path("resources-local-file-backend");
    std::fs::create_dir_all(&source_dir)?;
    std::fs::write(source_dir.join("hello.txt"), "hello from file backend")?;

    let canonical_source = std::fs::canonicalize(&source_dir)?;
    let backend = LocalFileBackend::new(vec![MountedResource {
        host_path: canonical_source.clone(),
        mount_path: PathBuf::from("/workspace"),
        read_only: true,
    }]);

    let bytes = backend
        .read(std::path::Path::new("/workspace/hello.txt"))
        .await
        .context("LocalFileBackend::read should translate mount path to host path")?;

    assert_eq!(
        bytes, b"hello from file backend",
        "INVARIANT (Resources): backend read returns the seeded bytes"
    );

    let _ = std::fs::remove_dir_all(&source_dir);
    Ok(())
}

/// Precondition: a runtime has been provisioned with a `LocalPath` resource
/// mounted at `/workspace` and an agent (scripted) that will run a shell
/// command like `cat /workspace/hello.txt` as a tool call.
///
/// Action: spawn the runtime, wait for provision, send a prompt that the
/// scripted agent interprets as "run cat on the mounted path", observe the
/// tool call result in the durable state stream.
///
/// Observable evidence: the tool call result contains the seeded file
/// contents, proving that the mounted path is visible to shell operations
/// inside the runtime container/process — NOT just visible at the
/// component layer.
///
/// Invariant proven: **Resources shell-visible physical mount** — the
/// contract Anthropic's post specifies, and the one the current
/// LocalPathMounter + LocalFileBackend tests do NOT prove. Shell-based
/// agents like Claude Code and Codex read files via bash/python/etc., which
/// bypasses the ACP fs protocol. The only faithful proof that Resources
/// works for those agents is a shell-visible read inside the runtime.
#[tokio::test]
#[ignore = "covered end-to-end by tests/control_plane_docker.rs slice 13c — this \
            stub is a primitive-coverage cross-reference marker for the \
            managed-agent mapping. The shell-visible-mount invariant only holds \
            under a container filesystem; local runtimes have no container fs to \
            prove it against. Not pending work; do not promote."]
async fn resources_physical_mount_is_shell_visible_inside_runtime() -> Result<()> {
    pending_contract(
        "resources.shell_visible_physical_mount",
        "Covered end-to-end by tests/control_plane_docker.rs slice 13c. This \
         stub is a primitive-coverage cross-reference marker for the managed-agent \
         mapping, not pending work. The shell-visible-mount invariant only holds \
         under a container filesystem; local runtimes have no container fs to prove \
         it against. Do not promote.",
    )
}

/// Precondition: a runtime has been provisioned with an
/// `FsBackendComponent` in its topology backed by the
/// `runtime_stream` backend, running the scripted `fireline-testy-fs`
/// agent that deterministically emits `fs/write_text_file` on
/// command.
///
/// Action: spawn the runtime, create a session, and prompt the agent
/// with the JSON command `{"command":"write_file","path":"/scratch/out.md","content":"..."}`.
/// The scripted agent parses the command and issues an ACP
/// `fs/write_text_file` request against its client connection.
/// Fireline's `FsBackendComponent` intercepts the request, writes
/// through the `StreamFsFileBackend`, and emits an `fs_op`
/// envelope to the durable state stream.
///
/// Observable evidence: an `fs_op` envelope is visible on the stream
/// with the expected path and content — proving the ACP fs
/// interception routes the write through the backend and captures
/// it as a Session event via `appendToSession`.
///
/// Invariant proven: **Resources ACP fs interception + artifact
/// capture** — the mapping doc §5 "Composable half" contract. Every
/// `fs/write_text_file` call is both routed to a backend AND durably
/// captured as an `fs_op` event.
#[tokio::test]
async fn resources_fs_backend_captures_write_as_durable_event() -> Result<()> {
    let topology = TopologySpec {
        components: vec![TopologyComponentSpec {
            name: "fs_backend".to_string(),
            config: Some(serde_json::json!({ "backend": "runtime_stream" })),
        }],
    };
    let spec = ManagedAgentHarnessSpec::new("resources-fs-backend-end-to-end")
        .with_agent_command(vec![testy_fs_bin().display().to_string()])
        .with_topology(topology);
    let runtime = LocalRuntimeHarness::spawn_with(spec).await?;

    let result = async {
        let session_id = create_session(runtime.acp_url()).await?;
        let prompt = serde_json::json!({
            "command": "write_file",
            "path": "/scratch/artifact.md",
            "content": "hello from fireline-testy-fs",
        })
        .to_string();
        prompt_session(runtime.acp_url(), &session_id, &prompt)
            .await
            .context(
                "INVARIANT (Resources): scripted testy-fs must accept the write_file command",
            )?;

        let fs_ops = wait_for_event_count(runtime.state_stream_url(), "fs_op", 1, DEFAULT_TIMEOUT)
            .await
            .context(
                "INVARIANT (Resources): the FsBackendComponent must capture fs/write_text_file \
             as a durable fs_op envelope on the state stream",
            )?;

        let fs_op = fs_ops
            .into_iter()
            .find(|env| {
                env.value()
                    .and_then(|v| v.get("path"))
                    .and_then(|v| v.as_str())
                    == Some("/scratch/artifact.md")
            })
            .context("fs_op for /scratch/artifact.md must be present on the stream")?;

        let content = fs_op
            .value()
            .and_then(|v| v.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert_eq!(
            content, "hello from fireline-testy-fs",
            "INVARIANT (Resources): captured fs_op content must match what the agent wrote"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

/// Precondition: a durable state stream is live (backed by a
/// `LocalRuntimeHarness` here; in production it would be any shared
/// durable-streams deployment).
///
/// Action: construct two independent `StreamFsFileBackend`
/// instances that both point at the same stream URL — simulating two
/// separate runtime processes attaching the same `SessionLogFileBackend`
/// to the same stream. Write via backend A, then read via backend B.
///
/// Observable evidence: backend B's read returns the bytes backend A
/// wrote, with no coordination between the two backend instances other
/// than sharing the stream URL.
///
/// Invariant proven: **Resources cross-runtime virtual filesystem** —
/// the `SessionLogFileBackend` special case in the mapping doc §5: a
/// file written via the backend is a Session event, and any consumer
/// reading the same stream can project the same file content. The
/// stream IS the filesystem, cross-runtime for free.
///
/// Scope note: this test proves the backend-level invariant (same
/// stream URL ⇒ same file surface). The "agent A emits
/// fs/write_text_file through its topology" end-to-end path still
/// requires a scripted agent and is covered by
/// `resources_fs_backend_captures_write_as_durable_event` (still
/// pending on scripted testy).
#[tokio::test]
async fn resources_session_log_backend_supports_cross_runtime_reads() -> Result<()> {
    let runtime = LocalRuntimeHarness::spawn("resources-cross-runtime-virtual-fs").await?;

    let result = async {
        let stream_url = runtime.state_stream_url().to_string();
        let backend_a = StreamFsFileBackend::new(stream_url.clone());
        let backend_b = StreamFsFileBackend::new(stream_url);

        let path = Path::new("/scratch/cross-runtime.txt");
        let payload = b"hello from backend A, visible to backend B via the shared stream";

        backend_a
            .write(path, payload)
            .await
            .context("backend A must accept an fs write to the shared stream")?;

        let bytes = backend_b.read(path).await.context(
            "INVARIANT (Resources): backend B pointed at the same stream URL must project \
                 the file written by backend A without any additional coordination",
        )?;

        assert_eq!(
            bytes, payload,
            "INVARIANT (Resources): cross-runtime read returns the bytes the other runtime wrote"
        );

        Ok(())
    }
    .await;

    runtime.shutdown().await?;
    result
}

// Suppress unused-import warnings for items we reference only in the
// commented oracle text of pending tests.
#[allow(dead_code)]
fn _referenced_imports(_: Arc<dyn FileBackend>) {}
