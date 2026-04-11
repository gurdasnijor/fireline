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

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use fireline_components::fs_backend::{FileBackend, LocalFileBackend};
use fireline_conductor::runtime::{LocalPathMounter, MountedResource, ResourceMounter, ResourceRef};
use managed_agent_suite::{pending_contract, temp_path};

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
            &ResourceRef::LocalPath {
                path: source_dir.clone(),
                mount_path: PathBuf::from("/workspace"),
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
        bytes,
        b"hello from file backend",
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
#[ignore = "pending: scripted testy harness (to deterministically trigger a bash tool \
            call against the mount) + ControlPlaneHarness or LocalRuntimeHarness \
            support for launching with resources passed through the launch spec"]
async fn resources_physical_mount_is_shell_visible_inside_runtime() -> Result<()> {
    pending_contract(
        "resources.shell_visible_physical_mount",
        "This is the Resources contract Anthropic's post actually specifies for \
         shell-based agents. Blocks on (1) scripted testy so the agent \
         deterministically executes `cat /workspace/hello.txt` as a tool call, and \
         (2) LocalRuntimeHarness or ControlPlaneHarness accepting ResourceRef in the \
         launch config and wiring it through BootstrapConfig.mounted_resources so the \
         runtime actually sees the mount. The current component-layer tests prove the \
         mounter and backend composition; this test would prove the end-to-end \
         contract against a real shell-reading agent.",
    )
}

/// Precondition: a runtime has been provisioned with an `FsBackendComponent`
/// in its topology backed by a `SessionLogFileBackend`, and an agent
/// (scripted) that will emit `fs/write_text_file` as an ACP effect.
///
/// Action: spawn the runtime, wait for provision, send a prompt that the
/// scripted agent interprets as "write /scratch/out.md with content X",
/// observe the state stream.
///
/// Observable evidence: an `fs_op` event for path `/scratch/out.md` appears
/// on the state stream with the expected content, and a subsequent
/// `fs/read_text_file` against the same path returns the same content —
/// proving the ACP fs interception routes both read and write through the
/// backend and captures the write as a Session event via
/// `appendToSession`.
///
/// Invariant proven: **Resources ACP fs interception + artifact capture** —
/// the mapping doc §5 "Composable half" contract. Every `fs/write_text_file`
/// call is both routed to a backend AND durably captured as an fs_op event,
/// which is the foundation for materialized artifact indexes.
#[tokio::test]
#[ignore = "pending: scripted testy harness (to deterministically emit fs/write_text_file) \
            + SessionLogFileBackend wired through a runtime launch config"]
async fn resources_fs_backend_captures_write_as_durable_event() -> Result<()> {
    pending_contract(
        "resources.fs_backend_captures_writes",
        "Blocks on scripted testy for deterministic fs/write_text_file emission and on \
         launching a runtime with an FsBackendComponent + SessionLogFileBackend attached \
         via topology. The component-layer test \
         (managed_agent_primitives_suite::managed_agent_resources_fs_backend_component_test) \
         already covers the FsBackendComponent implementation; this test covers the \
         runtime-attached end-to-end contract.",
    )
}

/// Precondition: two runtimes have been provisioned that share the **same**
/// Session stream (either both pointed at a single control-plane stream, or
/// both reading the same stream URL), and runtime A has written a file via
/// its `FsBackendComponent` + `SessionLogFileBackend`.
///
/// Action: call `backend.read` from runtime B's test-side against the same
/// path runtime A wrote, with runtime B's backend pointing at the shared
/// stream URL.
///
/// Observable evidence: runtime B's read returns the content runtime A wrote,
/// without any coordination between the runtimes.
///
/// Invariant proven: **Resources cross-runtime virtual filesystem** — the
/// `SessionLogFileBackend` special case in the mapping doc §5: a file written
/// via the backend becomes a Session event, and any other consumer reading
/// the same stream can project the same file content. The stream IS the
/// filesystem, cross-runtime for free.
#[tokio::test]
#[ignore = "pending: ControlPlaneHarness support for launching two runtimes against the \
            same shared stream, plus scripted testy, plus the SessionLogFileBackend \
            configured as the backend for the FsBackendComponent in each runtime's topology"]
async fn resources_session_log_backend_supports_cross_runtime_reads() -> Result<()> {
    pending_contract(
        "resources.cross_runtime_virtual_fs",
        "The elegant special case from mapping doc §5. Blocks on ControlPlaneHarness \
         launching two runtimes against the same shared durable-streams deployment + \
         scripted testy emitting fs/write_text_file from the first runtime. Once both \
         land this is a small test: write from runtime A, read from runtime B's backend \
         pointed at the same stream, assert identical content.",
    )
}

// Suppress unused-import warnings for items we reference only in the
// commented oracle text of pending tests.
#[allow(dead_code)]
fn _referenced_imports(_: Arc<dyn FileBackend>) {}
