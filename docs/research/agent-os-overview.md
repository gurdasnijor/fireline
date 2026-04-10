# agentOS (rivet-dev/agent-os) — Architecture Analysis

> Reference writeup of the [rivet-dev/agent-os](https://github.com/rivet-dev/agent-os) repository (main, April 2026).
> Written as a prior-art study while designing a similar system along a different axis.
> All file references are to paths inside `rivet-dev/agent-os`, not this repo.

## 1. TL;DR

agentOS is a self-hosted in-process "operating system" for AI coding agents (Claude Code, Codex, Pi, OpenCode, Amp). Its public surface is an npm package:

```ts
const vm = await AgentOs.create({ software: [common, pi] });
const { sessionId } = await vm.createSession("pi", { env: { ANTHROPIC_API_KEY } });
vm.onSessionEvent(sessionId, (event) => console.log(event));
await vm.prompt(sessionId, "Write a hello world script to /home/user/hello.js");
```

Underneath that friendly API is:

- A **Rust kernel** (`agent-os-kernel`) that models a POSIX-like OS — VFS, process table, FD table, pipes, PTYs, mounts, permissions, socket tables.
- A **Rust "execution" plane** (`agent-os-execution`) that controls real subprocesses for three guest runtimes: Node.js (JS/TS), CPython/Pyodide, and WebAssembly-via-Node.
- A **Rust sidecar** (`agent-os-sidecar`) that composes kernel + execution into a CLI binary speaking a versioned JSON wire protocol.
- A **TypeScript client** (`@rivet-dev/agent-os`) that spawns the sidecar as a child process, frames JSON over its stdio, and exposes a `Kernel`-shaped proxy to the rest of the Node codebase.

The TS↔Rust interop is **not** NAPI, wasm-bindgen, or embedded V8. It is literally a child process, `BigEndian-u32 length + serde_json body` over pipes. The Rust and TS sides both implement the same versioned protocol schema.

---

## 2. Repo layout

```
agent-os/
├── crates/                # Rust
│   ├── bridge/            # pure contracts — traits + value types, no I/O
│   ├── kernel/            # in-process POSIX-like kernel for one VM
│   ├── execution/         # guest runtime controllers (Node, Python, WASM)
│   ├── sidecar/           # wire protocol + stdio server + mount plugins
│   └── sidecar-browser/   # alternate browser target of the sidecar
│
├── packages/              # TypeScript (pnpm workspace)
│   ├── core/              # @rivet-dev/agent-os — public SDK + sidecar client
│   ├── posix/             # POSIX shims / typescript plumbing
│   ├── secure-exec/       # legacy JS kernel (kept for reference)
│   ├── secure-exec-typescript/
│   ├── shell, dev-shell, playground, browser, python, registry-types
│
├── docs/ + AGENTS.md + CLAUDE.md
└── registry/              # published agent-os packages (pi, codex, coreutils, …)
```

The Rust side is a Cargo workspace. The TypeScript side is a pnpm/Turborepo workspace. They are built side-by-side from the same repo. The TS `packages/core/src/runtime.ts` hardcodes the path `target/debug/agent-os-sidecar` and rebuilds it on demand from TS by calling `cargo` when inputs under `crates/bridge`, `crates/execution`, `crates/kernel`, or `crates/sidecar` are newer than the binary.

---

## 3. Process model at runtime

A running agentOS application looks like this:

```
Host Node.js process  (the user's app, runs @rivet-dev/agent-os)
 └── child: agent-os-sidecar (Rust)            ← TS spawns this; length-prefixed JSON on stdio
      ├── in-process: kernel VM #1              (agent-os-kernel)
      ├── in-process: kernel VM #2
      ├── child: node (guest VM #1, JS runtime) ← spawned by execution crate
      ├── child: node (guest VM #2, WASM runtime, Node flags --allow-wasi, --wasm-max-mem-pages=N)
      ├── child: python (if using Pyodide guest)
      └── child: ACP agent (e.g. the Pi binary, Claude Code) — spawned inside a kernel VM
```

Key properties:

- **One sidecar process** hosts multiple kernel VMs. The sidecar is authenticated per-connection and multiplexes many sessions and VMs.
- **Each kernel VM** is a logical, in-memory OS: it has its own VFS, process table, permissions, mount table, resource accountant, etc. VMs do *not* share state.
- **Each guest runtime** is a real OS subprocess, not an embedded interpreter. A "JavaScript VM" is a dedicated `node` child process; a "WASM VM" is another `node` child with WASI/worker/child-process caps configured, plus `--wasm-max-mem-pages`. Python guests shell out to `python` running Pyodide.
- **Host Node talks to the sidecar only via stdio frames.** It never directly touches guest processes. The TypeScript `Kernel` interface is an RPC proxy that the sidecar backs.
- **ACP agents themselves** (e.g. the Pi coding agent) run as *guest processes inside a kernel VM* — the sidecar starts them via its kernel process table, and TS talks to them via stdin/stdout pipes tunneled through the sidecar.

---

## 4. Rust crates

### 4.1 `agent-os-bridge` — the contract layer

`crates/bridge/src/lib.rs` is pure traits and value types. `#![forbid(unsafe_code)]`. No serde. No I/O. Its job is to define *what a host must expose to the kernel*, independent of how that host is wired.

Core traits:

```rust
pub trait BridgeTypes { type Error; }

pub trait FilesystemBridge: BridgeTypes { fn read_file(&mut self, …) -> …; /* 14 methods */ }
pub trait PermissionBridge: BridgeTypes { fn check_filesystem_access(&mut self, …) -> …; … }
pub trait PersistenceBridge: BridgeTypes { fn load_filesystem_state(&mut self, …) -> …; … }
pub trait ClockBridge: BridgeTypes { fn wall_clock(&mut self, …) -> …; … }
pub trait RandomBridge: BridgeTypes { fn fill_random_bytes(&mut self, …) -> …; }
pub trait EventBridge: BridgeTypes { fn emit_structured_event(&mut self, …); … }
pub trait ExecutionBridge: BridgeTypes {
    fn create_javascript_context(&mut self, …) -> GuestContextHandle;
    fn create_wasm_context(&mut self, …) -> GuestContextHandle;
    fn start_execution(&mut self, …) -> StartedExecution;
    fn poll_execution_event(&mut self, …) -> Option<ExecutionEvent>;
    // …stdin/kill/etc
}

pub trait HostBridge:
    FilesystemBridge + PermissionBridge + PersistenceBridge
    + ClockBridge + RandomBridge + EventBridge + ExecutionBridge {}
```

All request/response types are plain structs (`ReadFileRequest { vm_id, path }`, `StartExecutionRequest { vm_id, context_id, argv, env, cwd }`, `ExecutionEvent::{Stdout, Stderr, Exited, GuestRequest}`, etc.). These are the canonical data model for every other layer in the system.

**Significance:** the bridge crate is the *seam*. The same kernel instance could theoretically run against a `LocalBridge` (real host `std::fs`, `nix::*`), an in-memory test bridge, or a remote bridge. The trait-level contract is not coupled to JSON, stdio, or even a process boundary.

### 4.2 `agent-os-kernel` — the in-process POSIX kernel

`crates/kernel/src/lib.rs` declares modules:

```
command_registry   device_layer   fd_table   kernel
mount_plugin       mount_table    overlay_fs
permissions        pipe_manager   poll
process_table      pty            resource_accounting
root_fs            user           vfs
```

The central type is `KernelVm` in `crates/kernel/src/kernel.rs`. It owns, per-VM:

- `RootFileSystem` — the layered root (copy-on-write overlay over a base Alpine-like snapshot).
- `MountTable` — named mounts stacked on top.
- `ProcessTable` — full POSIX process model: PIDs, parent/child, process groups, sessions, signals (SIGCHLD, SIGTERM, SIGWINCH, SIGPIPE, SIGSTOP/CONT/TSTP), zombies, `waitpid`.
- `FdTableManager` — per-process FD tables (0–255) with refcounted `FileDescription`s supporting `dup`/`dup2`, `O_NONBLOCK` bits per FD, `flock`/`FileLockManager` keyed by the open-file-description identity so dup/fork inheritance sees the same lock but separate `open()`s conflict.
- `PipeManager` — kernel pipes with 64KB buffers. `EPIPE` surfaces as `SIGPIPE` delivery in the kernel-level `fd_write`, not inside `PipeManager` itself.
- `PtyManager` — PTY master/slave pairs with a line discipline; PTY resize emits `SIGWINCH` from the `KernelVm` entrypoint.
- `ResourceAccountant` + `ResourceLimits` — bounds on filesystem bytes/inodes, processes, open FDs, pipes, PTYs, sockets, per-operation memory guards for `pread`, `fd_write`, merged spawn argv/env, `readdir` batches, and WASM fuel/memory/stack.
- `Permissions` — four domains (`fs`, `network`, `childProcess`, `env`), each a function returning `{allow, reason}`. Deny-by-default unless `Permissions::allow_all()`. Path permission checks resolve symlinks *first* to avoid TOCTOU.
- `CommandRegistry` / `CommandDriver` — drivers that know how to launch commands (node, python, wasm) inside the kernel's process model.
- `DeviceLayer` — `/dev/null`, `/dev/urandom`, `/dev/pts/*`, etc.

The VFS is a tiered, chunked design (`ChunkedVFS` = `FsMetadataStore` + `FsBlockStore`): small files inline in metadata, larger files split into chunks in a key-value block store. Device and `/proc` layers, plus the permission wrapper, compose on top. All layers implement a `VirtualFileSystem` trait.

Crucially, the kernel's invariants (from `CLAUDE.md`) are absolute:

1. **Every guest syscall goes through the kernel.** File I/O goes through the kernel VFS, not real `node:fs`. Networking goes through the kernel socket table, not real `node:net`. Process spawning goes through the kernel process table, not real `node:child_process`. DNS goes through the kernel's DNS resolver.
2. **No real host builtins.** When a guest does `require('fs')`, the module loader must return a kernel-backed polyfill or deny with `ERR_ACCESS_DENIED`. Never fall through.
3. **The host is an implementation detail.** `process.pid` is the kernel PID, `os.hostname()` is the kernel hostname, stack traces must not reveal host paths, `process.env` must not contain internal `AGENT_OS_*` control vars.
4. **Polyfills are ports, not wrappers.** A path-translating shim over real `fs` is not a polyfill.
5. **Control channels are out-of-band.** Don't sniff magic prefixes on stdout/stderr — use dedicated fds.
6. **Resource consumption is bounded.** Every guest-allocatable resource has a configurable `ResourceLimits` key.
7. **Permission checks use resolved paths.**
8. **The VM behaves like a standard Linux environment.**

CLAUDE.md also openly marks the current Node isolation model as "KNOWN DEFICIENT": today, guest Node runs as real host Node with an ESM loader that intercepts builtins, but many builtins still wrap real `node:fs`/`node:net`. A previous all-JS kernel (`@secure-exec/core` + `@secure-exec/nodejs`, deleted at commit `5a43882`) had full kernel-backed polyfills using SharedArrayBuffer + `Atomics.wait` for sync syscalls from worker threads. The Rust sidecar's stated direction is to port those polyfills onto kernel primitives.

### 4.3 `agent-os-execution` — guest runtime controllers

`crates/execution/Cargo.toml` is deliberately lean:

```toml
[dependencies]
agent-os-bridge = { path = "../bridge" }
nix            = { version = "0.29", features = ["fs"] }
serde          = { version = "1.0", features = ["derive"] }
serde_json     = "1"
```

**There is no `wasmtime`, `v8`, or `deno_core` dependency.** Guest runtimes are not embedded in Rust. Instead, the execution crate is a *process controller*. Its modules:

```
common   node_import_cache   node_process
runtime_support   javascript   python   wasm
benchmark
```

Each runtime module (`javascript.rs`, `python.rs`, `wasm.rs`) builds a hardened `std::process::Command` and owns the child's lifecycle. The common pattern:

- Locate `node` (or `python`) via `AGENT_OS_NODE_BINARY` or default `node`.
- `harden_node_command` sets flags like `--permission`, `--allow-fs-read=`, `--allow-fs-write=`, `--allow-wasi`, `--allow-worker`, `--allow-child-process`, `--disable-warning=SecurityWarning`, and for WASM guests `--wasm-max-mem-pages=N`.
- Strips dangerous host env keys: `DYLD_INSERT_LIBRARIES`, `LD_LIBRARY_PATH`, `LD_PRELOAD`, `NODE_OPTIONS`.
- Injects `AGENT_OS_*` control env vars (entrypoint, bootstrap module, guest argv, guest path mappings, virtual pid/ppid/uid/gid, sandbox root, compile cache path, frozen time, etc.).
- Creates out-of-band control pipes (`AGENT_OS_CONTROL_PIPE_FD`) and, for sync RPC, separate request/response FDs so magic stdout prefixes aren't required.
- Pre-warms via an import cache (`NodeImportCache`) so cold-start hits the ~6 ms figure advertised in the README: Node's `--compile-cache`, a pre-resolved module graph, and a warm-up marker file.
- Spawns reader threads (`spawn_stream_reader`, `spawn_node_control_reader`) that pipe stdout/stderr and control messages back to the caller over an mpsc channel.

The execution crate exports typed events via the bridge's `ExecutionEvent` enum (Stdout / Stderr / Exited / GuestRequest). `GuestRequest` is the kernel-bound syscall RPC: a guest that wants to do `fs.readFile` serializes a sync RPC into an FD, the Rust execution controller reads it, forwards it into the kernel VM, and writes the response back on the matching FD. That's the execution equivalent of the `HostBridge` path, but in the reverse direction (guest → kernel).

So the architecture of execution is: **Rust Node.js process supervisor with a kernel-RPC side channel**. It is not embedded V8, but a very opinionated process harness.

### 4.4 `agent-os-sidecar` — wire protocol + stdio server + mount plugins

`crates/sidecar/` composes everything:

```
src/
├── lib.rs               # scaffold, re-exports service
├── main.rs              # 6 lines — calls stdio::run()
├── stdio.rs             # polling stdin, framing stdout, dispatch loop
├── protocol.rs          # versioned JSON wire protocol + codec + ResponseTracker
├── service.rs           # NativeSidecar, dispatch, LocalBridge, lifecycle
├── host_dir_plugin.rs          # mount plugin: host directory
├── google_drive_plugin.rs      # mount plugin: Google Drive
├── s3_plugin.rs                # mount plugin: S3
└── sandbox_agent_plugin.rs     # mount plugin: bridge to a full sandbox (E2B / Daytona)
```

#### Wire protocol (`protocol.rs`)

```rust
pub const PROTOCOL_NAME: &str    = "agent-os-sidecar";
pub const PROTOCOL_VERSION: u16  = 1;
pub const DEFAULT_MAX_FRAME_BYTES: usize = 1024 * 1024; // 1 MiB cap

#[derive(Serialize, Deserialize)]
#[serde(tag = "frame_type", rename_all = "snake_case")]
pub enum ProtocolFrame {
    Request(RequestFrame),
    Response(ResponseFrame),
    Event(EventFrame),
}

pub struct RequestFrame {
    schema:     ProtocolSchema,   // {"name":"agent-os-sidecar","version":1}
    request_id: u64,
    ownership:  OwnershipScope,
    payload:    RequestPayload,
}
```

Every frame carries an `OwnershipScope`, which is one of:

```rust
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum OwnershipScope {
    Connection { connection_id: String },
    Session    { connection_id, session_id },
    Vm         { connection_id, session_id, vm_id },
}
```

That scope is how a single sidecar multiplexes connections → sessions → VMs and enforces that a response's scope matches the original request's scope (see `ResponseTracker::accept_response` — it returns `OwnershipMismatch` otherwise).

**Framing.** `NativeFrameCodec::encode` produces:

```
[u32 big-endian length][serde_json bytes]
```

It rejects frames larger than `max_frame_bytes`, and `decode` returns `LengthPrefixMismatch` if the declared length doesn't match the actual buffer length. Nothing fancier — no multi-frame streaming, no CBOR, no compression.

**Request/response payloads** are large `serde(tag = "type", rename_all = "snake_case")` enums:

```rust
enum RequestPayload {
    Authenticate(AuthenticateRequest),
    OpenSession(OpenSessionRequest),
    CreateVm(CreateVmRequest),
    DisposeVm(DisposeVmRequest),
    BootstrapRootFilesystem(…),
    ConfigureVm(ConfigureVmRequest),           // mounts, software, permissions, instructions, projected modules
    GuestFilesystemCall(GuestFilesystemCallRequest),
    SnapshotRootFilesystem(…),
    Execute(ExecuteRequest),
    WriteStdin, CloseStdin, KillProcess,
    FindListener, FindBoundUdp,
    GetSignalState, GetZombieTimerCount,
    // …
}
```

Responses are the symmetric enum with `Authenticated`, `SessionOpened`, `VmCreated`, `VmConfigured`, `GuestFilesystemResult`, `ProcessStarted`, `ListenerSnapshot`, `SignalState`, `Rejected`, etc. Events are `VmLifecycle`, `ProcessOutput`, `ProcessExited`, plus structured events/logs.

`ResponseTracker` validates every incoming response: `DuplicateRequestId`, `DuplicateResponse`, `UnmatchedResponse`, `OwnershipMismatch`, `ResponseKindMismatch`. It also retains a capped set of completed request IDs (`DEFAULT_COMPLETED_RESPONSE_CAP = 10_000`) to detect stale retries.

#### Stdio server (`stdio.rs`)

`stdio::run` is the actual CLI entry point of the `agent-os-sidecar` binary. Roughly:

```rust
let config = NativeSidecarConfig { compile_cache_root: Some(default()), ..default() };
let codec  = NativeFrameCodec::new(config.max_frame_bytes);
let mut sidecar = NativeSidecar::with_config(LocalBridge::default(), config)?;
let mut writer  = SharedWriter::new(codec.clone(), BufWriter::new(io::stdout()));

loop {
    poll(stdin, POLLIN|POLLHUP|POLLERR, IDLE_POLL_SLEEP)?;
    let frame = read_frame(&codec, &mut stdin)?;
    let ProtocolFrame::Request(req) = frame else { error };

    let DispatchResult { response, events } = sidecar.dispatch(req.clone())?;
    track_session_state(&response.payload, &mut active_sessions, &mut active_connections);

    writer.write_frame(&ProtocolFrame::Response(response))?;
    for event in events { writer.write_frame(&ProtocolFrame::Event(event))?; }
}
```

`nix::poll` is used so idle VMs don't burn CPU. The sidecar flushes after every dispatch and tracks active sessions/connections to clean up if stdin closes. `LocalBridge` is a `HostBridge` impl that backs every bridge trait onto real host OS APIs (`std::fs`, `nix::sys::signal::kill`, `nix::sys::wait::waitid`, `hickory_resolver::TokioResolver` for DNS, `std::net::{TcpListener, UdpSocket}`, `std::os::unix::net::{UnixListener, UnixStream}`, `base64::Engine` for encoding, etc.).

#### Service (`service.rs`)

`NativeSidecar` is the dispatcher. It owns:

- A `SharedBridge<B>` — an `Arc<Mutex<B>>` over a `HostBridge` plus a permissions cache per-VM so permission decisions are consistent across concurrent calls.
- A `FileSystemPluginRegistry` seeded with mount plugin factories (`HostDirMountPlugin`, `S3MountPlugin`, `GoogleDriveMountPlugin`, `SandboxAgentMountPlugin`).
- A map of `connection_id → SessionState → VmState`.
- A per-VM `KernelVm<MountTable>` with a wired-up `CommandRegistry`, resource limits, and permission set.
- Execution engines: `JavascriptExecutionEngine`, `PythonExecutionEngine`, `WasmExecutionEngine`.

`dispatch(RequestFrame) -> DispatchResult { response, events }` is the big `match payload { … }`. It reconciles mounts *before* applying `payload.permissions` on `ConfigureVm`, so mount-time policy checks (e.g. `fs.mount_sensitive` for `/`, `/etc`, `/proc`) have to be present on the VM before `ConfigureVm` runs. `DisposeVm` sends SIGTERM, sleeps `DISPOSE_VM_SIGTERM_GRACE = 100ms`, then SIGKILL.

#### Mount plugins

`FileSystemPluginFactory` lets the sidecar treat mounts generically. `HostDirMountPlugin` bridges to a real host directory (with path normalization & permission checks); `S3MountPlugin` and `GoogleDriveMountPlugin` bridge to cloud storage; `SandboxAgentMountPlugin` talks to an external sandbox (E2B, Daytona) so you can "mount" a full Linux sandbox into a kernel VM's VFS on demand. This is how agentOS claims to coexist with sandboxes rather than compete with them.

---

## 5. TypeScript side

### 5.1 `packages/core/src/sidecar/`

```
client.ts                   # high-level AgentOsSidecar types, session lifecycle
handle.ts                   # managed handle abstraction
in-process-transport.ts     # alternative transport used in tests (no stdio)
native-process-client.ts    # the actual stdio wire client that spawns the Rust binary
native-kernel-proxy.ts      # adapter that presents a Kernel interface over the wire client
mount-descriptors.ts
permission-descriptors.ts
root-filesystem-descriptors.ts
```

#### `native-process-client.ts`

Mirrors the Rust protocol types in TypeScript discriminated unions:

```ts
const PROTOCOL_SCHEMA = { name: "agent-os-sidecar", version: 1 } as const;

type OwnershipScope =
  | { scope: "connection"; connection_id: string }
  | { scope: "session";    connection_id: string; session_id: string }
  | { scope: "vm";         connection_id: string; session_id: string; vm_id: string };

interface RequestFrame { frame_type: "request";  schema: …; request_id: number; ownership: …; payload: …; }
interface ResponseFrame { frame_type: "response"; … }
interface EventFrame    { frame_type: "event";    … }
```

`NativeSidecarProcessClient.spawn({ cwd, command, args })` runs `child_process.spawn`. Default is `cargo run -q -p agent-os-sidecar`, but `runtime.ts` rebuilds the binary once and then spawns it directly.

On the hot path it:

1. Accumulates `stdout` into a Buffer.
2. `drainFrames()` parses `[u32 length][json]` frames.
3. Responses resolve the matching `pendingResponses.get(request_id)` promise.
4. Events are dispatched to `eventWaiters` (selective listeners) and/or a shared event buffer.
5. Stderr is accumulated separately and attached to errors if the sidecar dies.
6. Typed helpers (`authenticateAndOpenSession`, `createVm`, `configureVm`, `bootstrapRootFilesystem`, `startProcess`, `writeStdin`, `killProcess`, `findListener`, `getSignalState`, …) wrap `sendRequest` and narrow the returned discriminated union, throwing if the response type doesn't match.

There's a `frameTimeoutMs` (default 60s) enforced per request.

#### `native-kernel-proxy.ts`

`NativeSidecarKernelProxy` implements the existing `Kernel` / `VirtualFileSystem` TypeScript interfaces declared in `runtime-compat.ts`, so everything else in `@rivet-dev/agent-os` (agents, sessions, ACP client, host tools, cron, MCP integration) can use `proxy.readFile(…)`, `proxy.spawn(…)`, `proxy.openShell(…)` without knowing there's a Rust sidecar at all. Every call is translated into a sidecar request. Process handles become synthetic IDs starting at `SYNTHETIC_PID_BASE = 1_000_000`. The proxy maintains:

- guest path mappings via `AGENT_OS_GUEST_PATH_MAPPINGS`
- extra fs read/write allow-lists via `AGENT_OS_EXTRA_FS_READ_PATHS` / `..._WRITE_PATHS`
- allowed Node builtins via `AGENT_OS_ALLOWED_NODE_BUILTINS` (defaults include `assert`, `buffer`, `child_process`, `console`, `crypto`, `dns`, `events`, `fs`, `http`, `http2`, `https`, `os`, `path`, `querystring`, `stream`, `string_decoder`, `timers`, `tls`, `url`, `util`, `zlib`)
- loopback-exempt ports via `AGENT_OS_LOOPBACK_EXEMPT_PORTS`
- an event pump with `EVENT_PUMP_TIMEOUT_MS = 86_400_000` (1 day — effectively "never time out")

The point: the existing TS kernel abstraction was preserved so the system could migrate from an all-JS kernel to a Rust sidecar without rewriting the higher layers.

#### `in-process-transport.ts`

A second implementation of the `AgentOsSidecarTransport` interface that skips the child process entirely, for tests. It proves the protocol is the contract, not the transport — you can swap the wire without touching the higher TS layers. This is the equivalent of having an "inproc" transport for a gRPC service.

### 5.2 `packages/core/src/runtime.ts`

Startup glue:

```ts
const REPO_ROOT      = fileURLToPath(new URL("../../..", import.meta.url));
const SIDECAR_BINARY = path.join(REPO_ROOT, "target/debug/agent-os-sidecar");
const SIDECAR_BUILD_INPUTS = [
  path.join(REPO_ROOT, "Cargo.toml"),
  path.join(REPO_ROOT, "Cargo.lock"),
  path.join(REPO_ROOT, "crates/bridge"),
  path.join(REPO_ROOT, "crates/execution"),
  path.join(REPO_ROOT, "crates/kernel"),
  path.join(REPO_ROOT, "crates/sidecar"),
];
```

On first use it stats each input, compares mtimes against `SIDECAR_BINARY`, and `execFileSync("cargo", ["build", "-p", "agent-os-sidecar"])` if the binary is stale. Then it spawns the binary, hands the process client to `NativeSidecarKernelProxy`, and presents the proxy as the `Kernel` to the rest of the TS codebase.

This file also defines the "Alpine-like" defaults (`KERNEL_POSIX_BOOTSTRAP_DIRS`, `/bin`, `/etc`, `/home/user`, …), `KERNEL_COMMAND_STUB = "#!/bin/sh\n# kernel command stub\n"`, and a whole `VirtualFileSystem` interface definition so its shape stays in lockstep with the Rust kernel's.

### 5.3 Public SDK: `packages/core/src/agent-os.ts`

This is the class users see: `AgentOs.create(options)`. Its job is to wire up all the TS subsystems against the sidecar-backed kernel:

- `AcpClient` — a JSON-RPC client that speaks ACP (the Agent Communication Protocol).
- `Session` / `session.ts` — tracks session lifecycle, mode state, capabilities, permission requests, sequenced notifications, pending tool calls.
- `host-tools.ts`, `host-tools-argv.ts`, `host-tools-server.ts`, `host-tools-shims.ts`, `host-tools-prompt.ts` — let you define JS host tools via `hostTool({ description, inputSchema: z.…, execute })`, expose them as guest CLI commands inside the VM (shims generated in the VFS), route their RPCs to an HTTP server on the host, and auto-generate prompt docs.
- `cron/` — scheduled tasks backed by a `ScheduleDriver` (default `TimerScheduleDriver`).
- `CronManager`, `filesystem-snapshot.ts`, `host-dir-mount.ts`, `layers.ts`, `overlay-filesystem.ts`, `packages.ts`, `agents.ts`, `base-filesystem.ts`, `sqlite-bindings.ts`, `stdout-lines.ts`.
- MCP integration: `McpServerConfig` is `{type: "local", command, args, env}` or `{type: "remote", url, headers}`, attached per session.

`CreateSessionOptions` includes `cwd`, `env`, `mcpServers`, `skipOsInstructions`, `additionalInstructions`. OS instructions are written to `/etc/agentos/instructions.md` inside the VM.

`CreateOptions` includes `software`, `moduleAccessCwd`, `rootFilesystem`, `mounts`, `additionalInstructions`, `scheduleDriver`, `toolKits`, `permissions`, and `sidecar` (placement config — shared pool by default, or pin to a specific sidecar handle).

### 5.4 ACP layering — `acp-client.ts` and `session.ts`

`AcpClient` is a JSON-RPC client over a `ManagedProcess`'s stdin/stdout (line-delimited JSON-RPC, with `_stdoutIterator` reading newline-separated frames). Key behaviors:

- **Bidirectional.** Handles incoming requests from the agent (via `InboundRequestHandler`) as well as outgoing `request(method, params)` calls with a `_pending` map keyed by request id.
- **Permission translation.** The `ACP_PERMISSION_METHOD = "session/request_permission"` (plus a legacy `request/permission`) is intercepted; pending permission requests are tracked in `_pendingPermissionRequests` and later resolved by the host when the user answers.
- **Cancellation.** `ACP_CANCEL_METHOD = "session/cancel"`.
- **Activity log.** Keeps a bounded ring (`RECENT_ACTIVITY_LIMIT = 20`) of request/response/notification summaries for debugging.
- **Timeouts.** `DEFAULT_TIMEOUT_MS = 120_000`, with `EXIT_DRAIN_GRACE_MS = 50` so pending frames can be drained when the subprocess exits.

The important bootstrap nuance: `AcpClient` is **not** a discovery client. It
does not resolve URLs or create runtimes. It assumes a `ManagedProcess`
already exists. `runtime.ts` / `AgentOs.create(...)` own sidecar spawn, VM
creation, and ACP process bootstrap; `AcpClient` simply speaks ACP over that
already-owned transport.

`Session` sits above the ACP client and tracks:

- Sequenced event history (every notification gets a monotonically increasing `sequenceNumber` so clients can resume `onSessionEvent` without drops).
- `SessionModeState`, `SessionConfigOption[]`, `AgentCapabilities` (permissions, plan_mode, tool_calls, text_messages, images, streaming_deltas, mcp_tools, etc.), `AgentInfo`.
- Permission request handlers.

The important architectural observation: the ACP agent binary (Pi, Claude Code, Codex, …) is a guest *inside* the kernel VM. `Session` talks to it via `ManagedProcess`, which is actually `NativeSidecarKernelProxy`'s synthetic process — its stdin/stdout stream through the sidecar's `WriteStdin`/`ProcessOutput` events. So: **JSON-RPC over a virtual pipe over a framed JSON protocol over a real host pipe**. Three layers of framing, but each with a clear purpose.

---

## 6. The flow of a single call, end to end

User code: `const data = await vm.readFile("/home/user/hello.js");`

1. `AgentOs.readFile` → `NativeSidecarKernelProxy.readFile("/home/user/hello.js")`.
2. Proxy calls `NativeSidecarProcessClient.sendRequest`:
   ```json
   {
     "frame_type": "request",
     "schema": {"name": "agent-os-sidecar", "version": 1},
     "request_id": 42,
     "ownership": {"scope": "vm", "connection_id": "c-1", "session_id": "s-1", "vm_id": "v-1"},
     "payload": {
       "type": "guest_filesystem_call",
       "operation": "read_file",
       "path": "/home/user/hello.js"
     }
   }
   ```
3. Client serializes with `JSON.stringify`, prepends a big-endian `u32` length, writes to the sidecar child's `stdin`.
4. `stdio::run` on the sidecar side wakes up from `poll()`, reads the framed bytes, decodes into a `ProtocolFrame::Request`.
5. `NativeSidecar::dispatch(req)` matches `GuestFilesystemCall`, looks up the `KernelVm` for `vm_id: v-1`, resolves the path through its VFS (with permission wrapper), calls the VFS's `read_file`. The permission bridge is invoked via `SharedBridge`, which short-circuits cached decisions and records new ones.
6. Dispatch returns `DispatchResult { response, events }` with `payload = GuestFilesystemResult { operation: "read_file", path, content: base64(bytes), encoding: "base64" }` and any structured events generated during the call.
7. `stdio::run` writes the response frame and any trailing event frames back to stdout.
8. The TS client's `stdout.on("data", …)` accumulates bytes, `drainFrames()` extracts the length-prefixed frames, looks up `pendingResponses.get(42)`, narrows the discriminated union, resolves the promise.
9. Proxy decodes base64 to a `Uint8Array`, returns to the caller.

For `vm.exec("node /hello.mjs")`, the flow is similar but the call becomes `ExecuteRequest` → `KernelVm::spawn` → the kernel's `CommandRegistry` selects the node driver → `JavascriptExecutionEngine` in the execution crate builds a hardened `Command`, installs the control-channel FDs, and spawns a real `node` process. Subsequent `ProcessOutput` and `ProcessExited` events are pushed back over the sidecar's event stream until the process exits. The TS proxy then synthesizes a `KernelExecResult`.

---

## 7. Filesystem model

From `docs/filesystem.mdx`, three concepts compose:

- **Base filesystem** — a pre-built Alpine-like snapshot bundled with the runtime (`/bin`, `/etc`, `/home/user`, hostname `agent-os`, `PATH`, `PAGER`, symlinks, standard `/etc` files). Generated in two steps: capture an Alpine snapshot, then normalize it for agentOS.
- **Writable overlay** — per-VM copy-on-write layer. Reads fall through to the base when untouched, writes land in the overlay (copy-up if the file exists below), deletes are whiteouts, new files are in the overlay only.
- **Mounted filesystems** — mount subtrees at specific paths (S3, host dir, Google Drive, layer-store overlays, sandbox-agent mounts). Mounts *replace* subtrees: `/data` on a mount resolves to the mounted FS, not the overlay.

Lookup order: mount → overlay → base. `mode: "read-only"` drops the overlay entirely and reads directly from the merged lower stack. `AgentOs.snapshotRootFilesystem()` exports the current visible root as a reusable snapshot that can be fed back as `rootFilesystem.lowers` for later VMs.

The snapshot format is versioned (`ROOT_FILESYSTEM_SNAPSHOT_FORMAT`) in the kernel crate. The sidecar has `encode_snapshot` / `decode_snapshot` helpers so snapshots can cross the wire.

---

## 8. Security & isolation model (intent vs. current reality)

**Intent** (from CLAUDE.md and the kernel traits):

- Deny-by-default `Permissions` in four domains: `fs`, `network`, `childProcess`, `env`. Every kernel syscall checks policy.
- Permission checks resolve symlinks first; `link()` checks both source and destination.
- Sensitive mount policy: mounts targeting `/`, `/etc`, `/proc` require a distinct `fs.mount_sensitive` permission in addition to normal `fs.write` on the mount path.
- Resource caps live in `ResourceLimits` for every allocatable resource, including per-operation memory guards on `pread`, `fd_write`/`fd_pwrite`, merged spawn argv/env, `readdir` batches, and WASM fuel/memory/stack.
- Kernel VM configs opt into broad access explicitly: `KernelVmConfig::new()` is deny-all; tests and browser scaffolds must set `Permissions::allow_all()` themselves.
- Out-of-band control channels — no magic stdout prefixes (to prevent guest spoofing of control messages). Dedicated FDs or `waitid(WNOWAIT|WNOHANG|…)` style probes.
- No host-path leakage via stack traces or `process.env`.
- WebAssembly parser hardening: stat modules before `fs::read`, cap import/memory section entry counts, bound varuint byte length, fail-closed on malformed or oversized modules.
- POSIX correctness: correct `errno` values, proper signal delivery (`SIGCHLD`, `SIGTERM`, `SIGPIPE` from `fd_write`, `SIGWINCH` from PTY resize, job control `SIGSTOP`/`SIGCONT`/`SIGTSTP`), standard `/proc` layout, expected filesystem behavior.

**Current reality** (per the Node.js Isolation Model section of CLAUDE.md): guest Node runs as a real host Node.js child. The ESM loader hooks intercept `require()`/`import`, but only a subset of builtins have true kernel-backed polyfills. The status table:

| Builtin | Required | Current | Gap |
|---|---|---|---|
| `fs` / `fs/promises` | kernel VFS polyfill | path-translating wrapper over real `node:fs` | port to kernel VFS over RPC |
| `child_process` | kernel process table polyfill | wrapper over real `node:child_process` | port to kernel process table |
| `net`, `dgram`, `dns`, `http`, `https`, `http2` | kernel socket/DNS polyfills | fall through to real host modules | port the socket table polyfills |
| `tls` | kernel TLS polyfill | guest-owned polyfill that wraps guest net transport with host TLS state | keep TLS on guest sockets |
| `os` | kernel-backed values | guest polyfill virtualizing hostname, CPU, memory, loopback net, home, user info | align with VM defaults |
| `vm` | must be denied | falls through | deny |

The intended direction is to port the polyfills that the previous all-JS kernel (`@secure-exec/core` + `@secure-exec/nodejs`) already had — using SharedArrayBuffer + `Atomics.wait` on a request/response pair for sync syscalls from worker threads, which is the same pattern the Pyodide VFS bridge uses today.

So: the Rust kernel is architecturally complete and the data path is ready, but the Node guest-side polyfill layer that actually routes `require('fs')` into the kernel VFS is still being finished. Worth knowing if you're evaluating whether "V8 isolates" actually holds today.

---

## 9. Interop patterns worth stealing (or avoiding)

### Patterns that carry their weight

1. **Traits-first bridge crate, decoupled from transport.** `agent-os-bridge` has no serde, no I/O, no `tokio`. It's a pure trait contract with plain data types. Every other layer either *implements* the traits (`LocalBridge`) or *consumes* them (`KernelVm`). The wire protocol is a separate concern in `agent-os-sidecar`. That made it cheap to add an `in-process-transport.ts` for tests without touching the kernel.

2. **Versioned, explicit wire schema with ownership scopes.** Every frame carries `{name, version}` and an `OwnershipScope`. `ResponseTracker` enforces ownership match, request kind match, and dedup via a bounded completed-ID set. This makes single-process multiplexing of many sessions safe without ad-hoc channel bookkeeping.

3. **A `Kernel` TypeScript interface that predated the Rust port.** `NativeSidecarKernelProxy` implements the same interface the JS kernel used to, so migrating from an all-JS kernel to a Rust sidecar did not force a rewrite of `AgentOs`, `Session`, `AcpClient`, or host tools. The Rust kernel's life is easier because it has a crisp external contract to meet.

4. **Side-channel FDs for sync RPC from guest workers.** `JavascriptSyncRpcChannels` exposes dedicated request/response pipes (`AGENT_OS_NODE_SYNC_RPC_REQUEST_FD` / `..._RESPONSE_FD`), with configurable data byte caps (`NODE_SYNC_RPC_DEFAULT_DATA_BYTES = 4 MiB`) and wait timeouts (`NODE_SYNC_RPC_DEFAULT_WAIT_TIMEOUT_MS = 30_000`). Keeps control traffic off stdout so guests can't spoof it. This is also a clean place to wire SharedArrayBuffer + `Atomics.wait` polyfills in the future.

5. **Auto-rebuild-on-change for the Rust binary.** `runtime.ts` stats the Cargo inputs, rebuilds the sidecar if any input is newer than the binary, then spawns. One repo, two languages, no manual build step for developers. This is a massive DX win if your binary lives next to the TS code.

6. **Mount plugin registry with a narrow trait.** `FileSystemPluginFactory` + `FileSystemPluginRegistry` lets new sources (S3, GDrive, sandbox agents, host dirs) be added without changing kernel code. The "sandbox extension" (mount a full E2B/Daytona sandbox as a subtree) is just another plugin.

7. **Sequenced events on sessions.** `SequencedEvent { sequenceNumber, notification }` and `GetEventsOptions` with "greater than this value" let multiple observers resume without duplication or loss. Pairs naturally with multiplayer.

8. **Ownership-scoped frames let you clean up cascades.** When a connection drops, the sidecar walks `active_sessions` and `active_connections` tracked in `stdio.rs` and runs session/VM disposal. One source of truth for cleanup.

### Patterns to approach with care

1. **"V8 isolates" in marketing, `node` subprocesses in practice.** Real isolation via NAPI-bound V8 or embedded wasmtime is very different from Node subprocess + flags. Cold start will be dominated by Node + your compile cache warmup, not V8 context creation. Whether this matters depends on your budget for "escape boundary." CLAUDE.md is refreshingly honest that the current Node guest is "KNOWN DEFICIENT."

2. **Framing at 1 MiB.** `DEFAULT_MAX_FRAME_BYTES = 1 MiB` is an inline file-read cap, so anything larger must be streamed via `Execute` + stdout events instead of `GuestFilesystemCall`. Design your file APIs with a streaming alternative from day one.

3. **Line-delimited JSON-RPC over a virtual pipe over a framed JSON protocol over a real host pipe.** Three framing layers means three places latency and bugs can hide. Be deliberate about which layer owns backpressure, timeouts, and cancellation. Here: layer 1 (agent ACP) owns request timeouts (`DEFAULT_TIMEOUT_MS = 120_000`), layer 2 (sidecar wire) owns frame timeouts (`frameTimeoutMs = 60_000`), layer 3 (real host pipe) is left to Node's default backpressure.

4. **`Arc<Mutex<B>>` SharedBridge.** A single mutex around the whole bridge serializes every kernel call through one lock. Fine for a handful of VMs, potentially contentious at scale. If you expect high concurrency, plan to shard the bridge per-VM or use fine-grained interior mutability.

5. **Auto-rebuild coupling.** The TS runtime assumes the Cargo workspace lives adjacent to the TS workspace with a specific `target/debug/...` path. Fine for monorepo-style development, awkward for publishing prebuilt binaries. They publish the sidecar as part of the npm package with a rebuild fallback; this is something you'd need to design for explicitly.

6. **Deny-by-default is the intent, but tests and scaffolds are wide open.** Look at every `Permissions::allow_all()` call site before you trust a benchmark or security claim.

---

## 10. Glossary / quick reference

- **`AgentOs`** — the TypeScript entry class.
- **Kernel VM** — one in-memory OS instance inside the sidecar (not a hypervisor VM). Has its own VFS, process table, permissions, mounts.
- **Session** — an ACP session bound to one or more VMs; has modes, capabilities, permission prompts, and sequenced event history.
- **Sidecar** — the Rust `agent-os-sidecar` binary. One process, many connections/sessions/VMs.
- **ACP** — Agent Communication Protocol, JSON-RPC 2.0, spec at agentclientprotocol.com.
- **Guest runtime** — Node.js, Python (Pyodide), or WebAssembly (via Node's WASM). Each is a child process hardened with Node `--permission`/`--allow-*` flags and `AGENT_OS_*` env vars.
- **`HostBridge`** — Rust super-trait = `FilesystemBridge + PermissionBridge + PersistenceBridge + ClockBridge + RandomBridge + EventBridge + ExecutionBridge`.
- **`LocalBridge`** — default `HostBridge` impl in `sidecar::service` that wires bridge traits to real `std::fs`, `nix`, `hickory_resolver`, etc.
- **`NativeFrameCodec`** — `[u32 BE length][serde_json body]` with a 1 MiB default cap.
- **`OwnershipScope`** — `{connection}`, `{connection, session}`, or `{connection, session, vm}` — carried on every frame.
- **`ResponseTracker`** — enforces ownership match, response-kind match, and dedup; retains last 10k completed request IDs.
- **`KernelVmConfig`** — per-VM config: `env`, `cwd`, `permissions`, `resources`, `vm_id`.
- **`ResourceLimits`** — caps on fs bytes, inodes, processes, FDs, pipes, PTYs, sockets, plus per-op memory guards and WASM fuel/memory/stack.
- **`NativeSidecarKernelProxy`** — the TS class that implements the `Kernel` interface over the wire client.
- **Host tools** — user-defined host-side JS functions exposed to the guest as CLI binaries (`agentos-{toolkit}`), invoked over an HTTP server the host spins up.

---

## 11. For our project: things worth contrasting against

Notes for the durable-acp-rs design:

- agentOS's bridge separation (`agent-os-bridge` is I/O-free) is a model worth copying when we want a transport-agnostic contract layer.
- Their `OwnershipScope` nested `connection → session → vm` maps cleanly onto our conductor → session → runtime hierarchy; we can steal the frame-scoped ownership pattern directly if we want multiplexing with correct cleanup cascades.
- They run ACP agents as *guests* inside a kernel VM and proxy JSON-RPC through the sidecar. Our architecture keeps ACP closer to the surface — worth articulating the tradeoff explicitly in our handoff docs.
- Their `in-process-transport.ts` / `NativeSidecarProcessClient` split is a good pattern for making integration tests deterministic without losing the stdio wire client as an integration surface.
- Their `AcpClient` is a transport wrapper over an already-owned process, not a bootstrap/discovery layer. If we want both hosted and local modes, our TS ACP primitive should likely split into `connect({ url })` and `attach({ process | transport })`, with runtime/bootstrap left to `client.host` / Flamecast.
- The sequenced-event + `GetEventsOptions` pattern on `Session` is useful prior art for how we model resumable trace streams in the state consumer.
- The "rebuild Rust binary if inputs are newer" DX hack in `runtime.ts` is worth considering if we end up shipping a Rust sidecar alongside a TS client in the same repo.
- Their TS↔Rust bridge is pipe-based JSON, not NAPI. Something to weigh when picking *our* bridge technology — simpler to reason about, easier to debug with plain logs, but every call pays a serialize/framing cost.
