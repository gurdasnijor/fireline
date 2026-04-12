# File-helper REST API usage

## Status

Dead. There are **0 live callers** for the browser/workspace file-helper
REST API, and the routes are not wired into runtime bootstrap.

## Evidence

- `grep -R -n "/api/v1/files" packages crates src` returns **2 hits**, both
  comments, not executable callers:
  - [`crates/fireline-resources/src/routes_files.rs:9`](../../crates/fireline-resources/src/routes_files.rs)
  - [`crates/fireline-session/src/stream_host.rs:16`](../../crates/fireline-session/src/stream_host.rs)
- `grep -R -n "routes::files\|routes_files\|FileHelper" packages crates src`
  finds no live route registration. The only code hits are:
  - the stub export in `crates/fireline-resources/src/lib.rs`
  - the TODO comment in
    [`crates/fireline-host/src/connections.rs:10`](../../crates/fireline-host/src/connections.rs)
  - the stub module itself in
    [`crates/fireline-resources/src/routes_files.rs`](../../crates/fireline-resources/src/routes_files.rs)
- `grep -R -n "connections/" packages/browser-harness` returns **0 hits**.
- [`packages/browser-harness/src/app.tsx`](../../packages/browser-harness/src/app.tsx)
  does not fetch any file-helper route. The only harness API fetches are:
  - `/api/agents` at line 392
  - `/api/resolve` at line 431
- The browser ACP client explicitly does **not** advertise file support:
  - `clientCapabilities: { fs: { readTextFile: false } }` at
    [`packages/browser-harness/src/app.tsx:271`](../../packages/browser-harness/src/app.tsx)
  - `readTextFile()` throws at
    [`packages/browser-harness/src/app.tsx:1005`](../../packages/browser-harness/src/app.tsx)
  - `writeTextFile()` throws at
    [`packages/browser-harness/src/app.tsx:1002`](../../packages/browser-harness/src/app.tsx)
- Runtime bootstrap does not wire the file-helper routes. The live app is:
  - `Router::new().route("/healthz", ...).merge(fireline_harness::routes_acp::router(...))`
    at [`crates/fireline-host/src/bootstrap.rs:199-202`](../../crates/fireline-host/src/bootstrap.rs)
  - no `routes_files` merge anywhere in `crates/fireline-host/src/`

Caller count:

- `/api/v1/files*` live callers: **0**
- browser-harness file-helper fetches: **0**
- bootstrap route merges for file-helper API: **0**

## Historical intent

The original design was a browser convenience REST API, not a core Host
or ACP requirement.

- [`crates/fireline-host/src/connections.rs`](../../crates/fireline-host/src/connections.rs)
  sketches a file-backed lookup table under
  `~/.local/share/fireline/runtime/connections/{id}.toml` so the server
  could map `connection_id -> cwd`.
- [`crates/fireline-resources/src/routes_files.rs`](../../crates/fireline-resources/src/routes_files.rs)
  sketches:
  - `GET /api/v1/files/{connection_id}`
  - `GET /api/v1/files/{connection_id}/tree`
- That same module comment already hints that REST was a stopgap and
  could eventually be replaced by an MCP/filesystem component instead.

No ACP contract in the repo requires this REST API. The ACP-facing file
contract referenced in the codebase is `fs/read_text_file` /
`fs/write_text_file`, not `/api/v1/files/*`.

## Current replacement

For the **browser harness UI itself**, there is no replacement because
there is no current feature that browses workspace files. The browser
does not call a file-helper REST endpoint and does not implement ACP
`readTextFile` / `writeTextFile` either.

For **live file operations elsewhere in Fireline**, the replacement is
already split across the current primitives:

- **ACP fs methods** for ACP-native agents:
  - [`crates/fireline-resources/src/fs_backend.rs`](../../crates/fireline-resources/src/fs_backend.rs)
    intercepts `ReadTextFileRequest` and `WriteTextFileRequest`
  - [`src/bin/testy_fs.rs`](../../src/bin/testy_fs.rs) is a live test
    agent that emits `fs/read_text_file` and `fs/write_text_file`
  - [`tests/managed_agent_resources.rs:175-239`](../../tests/managed_agent_resources.rs)
    exercises that path end to end
- **Resources mounts** for shell-based agents:
  - `LocalPathMounter` / mounted resources are the physical file path
    path for agents that use shell/python instead of ACP fs
  - this is the direction documented in
    [`docs/explorations/managed-agents-mapping.md:458-511`](../explorations/managed-agents-mapping.md)

So the old REST helper is not the active file path. The live file paths
are ACP fs for ACP-native agents and Resources mounts for shell-visible
files.

## Recommendation

**1. Delete both stubs outright.**

Rationale:

- there are no live callers
- there is no route registration
- the browser harness does not depend on the feature
- ACP already has a standard file contract for protocol-native file
  operations, and the repo already uses that path in `FsBackendComponent`
- the product direction for shell-visible files is Resources, not a
  side REST API

If a future UI needs file browsing, it should be re-specified against
ACP fs or an MCP/Resources-backed surface, not by reviving
`/api/v1/files/*`.

## Follow-up work

- Delete `crates/fireline-host/src/connections.rs`.
- Delete `crates/fireline-resources/src/routes_files.rs`.
- Remove the stale `routes_files` export from `crates/fireline-resources/src/lib.rs`.
- Fix stale comments/docs that still imply the helper routes exist:
  - `crates/fireline-session/src/stream_host.rs`
  - `docs/demo-runbook.md`
  - `docs/architecture.md`
