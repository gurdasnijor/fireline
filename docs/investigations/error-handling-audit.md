# Error Handling Audit

Date: 2026-04-11

Scope:
- Rust sources under `crates/` and `src/`
- Focused on panic paths in production code, silent error swallowing, missing context on operational failures, and data-loss patterns

Note:
- The requested skill file `~/.agents/skills/m06-error-handling/SKILL.md` was not present in this environment, so this audit was performed directly against the codebase.

## Summary

I found 10 production-relevant issues:
- 5 issues that can panic in production request/task paths
- 4 issues that can silently drop errors or durable-state writes
- 1 issue where the returned error loses the real operational cause after retries

No material P1/P2 resource-leak findings were confirmed in Category E. The long-lived spawned tasks I checked are either stored and aborted/joined explicitly or aborted on drop.

## Category A — `unwrap` / `expect` in non-test code

| Severity | File:line | Issue | Recommended fix |
|---|---|---|---|
| P1 | `crates/fireline-tools/src/peer/mcp_server.rs:169-176` | `prompt_peer` calls `.expect("parent lineage should always include an active turn id")` on request-path lineage data. If the active-turn projection lags or the lineage is malformed, the tool handler panics instead of returning an error. | Replace the `expect` with explicit validation and return a `sacp::util::internal_error(...)` that includes the missing lineage field. |
| P2 | `crates/fireline-harness/src/shared_terminal.rs:120-123` | `SharedTerminalAttachment::connect_to()` calls `.take().expect(...)` on `incoming_rx`. A double-connect or attachment reuse bug will panic inside async session setup and kill the task. | Return a transport/setup error instead of panicking, e.g. map the missing receiver to `io::ErrorKind::BrokenPipe` or a structured `sacp` internal error. |
| P2 | `crates/fireline-host/src/auth.rs:38-47`; `crates/fireline-sandbox/src/registry.rs:71-88` | Long-lived service state is guarded by `Mutex::lock().expect("... poisoned")`. Any earlier panic while holding the lock turns later auth or liveness operations into process/task panics. | Replace `expect` with recoverable handling: map poisoning to an error, log it, and either recover with `into_inner()` or fail the current operation cleanly. |
| P2 | `src/main.rs:547-550`; `crates/fireline-host/src/bootstrap.rs:412-415`; `crates/fireline-host/src/control_plane.rs:215-218`; `crates/fireline-host/src/router.rs:282-285`; `crates/fireline-orchestration/src/child_session_edge.rs:95-98`; `crates/fireline-resources/src/publisher.rs:155-158`; `crates/fireline-sandbox/src/lib.rs:358-361` | Multiple production `now_ms()` helpers use `.expect("time went backwards")`. A clock step backwards or broken host clock turns ordinary request/task work into a panic. | Standardize on a non-panicking clock helper. Either return `Result<i64>`, or clamp/fallback with a warning so callers keep running and the skew is observable. |
| P2 | `crates/fireline-sandbox/src/lib.rs:271-277` | `register()` still contains `unreachable!("stopped runtimes already returned above")`. If the status precondition drifts during future edits, the control-plane path panics instead of returning a normal error. | Replace `unreachable!` with an explicit error branch so status mismatches stay recoverable and visible. |

## Category B — Silently swallowed errors

| Severity | File:line | Issue | Recommended fix |
|---|---|---|---|
| P1 | `crates/fireline-resources/src/registry.rs:127-145`; `crates/fireline-resources/src/registry.rs:155-170` | `StreamResourceRegistry` drops projection failures silently. Reader build errors return with no log, stream read errors return with no log, malformed chunks are ignored with no log, and `index.apply(...)` errors are discarded. This can leave the resource registry permanently stale with no signal. | Log every failure with `stream_url` and relevant event metadata. Retry transient read failures instead of returning, and surface a health signal when the projection has stopped. |
| P1 | `crates/fireline-host/src/local_provider.rs:184-192` | When child runtime startup fails, cleanup is attempted with `let _ = runtime.try_shutdown().await;`. If shutdown itself fails, that failure is lost and the original startup error hides a leaked child process or half-cleaned runtime. | Preserve both failures: log the shutdown error and attach it as context to the original startup error. |
| P1 | `crates/fireline-harness/src/trace.rs:108-119`; `crates/fireline-orchestration/src/child_session_edge.rs:50-71` | Important durable-state writes are fire-and-forget only. `emit_host_instance_started()` and `ChildSessionEdgeWriter::emit_child_session_edge()` call `producer.append_json(...)` and return success without `flush()` or any error callback path. A fast shutdown or producer failure can silently drop source-of-truth records. | Make these writers return/await a flushed `Result`, or install and observe producer error callbacks so dropped writes are at least logged and counted. |
| P2 | `crates/fireline-harness/src/context.rs:193-199` | `WorkspaceFileSource::gather()` converts every file-read failure into `Ok(String::new())`. Permission errors, transient I/O failures, or unexpected path bugs silently remove prompt context with no observability. | Treat `NotFound` as the only silent-empty case. For other errors, log the path and error or propagate a structured `sacp::Error`. |

## Category C — Missing error context

| Severity | File:line | Issue | Recommended fix |
|---|---|---|---|
| P2 | `crates/fireline-host/src/control_plane_client.rs:41-83` | `register()` retries transport and HTTP failures, but the final error returned after 3 attempts is only `"registration failed ... after 3 attempts"`. The actual last status / transport cause is lost to the caller, making production failures hard to root-cause. | Carry the last failure forward. Include the final HTTP status and response body when available, or wrap the last transport error with `.context("control-plane registration")`. |

## Category D — Panic paths in async code

| Severity | File:line | Issue | Recommended fix |
|---|---|---|---|
| P1 | `crates/fireline-tools/src/peer/mcp_server.rs:169-176` | This `expect` sits inside an async tool handler. A bad lineage record kills the request task instead of producing an error response. | Return a structured handler error, not a panic. |
| P2 | `crates/fireline-harness/src/shared_terminal.rs:120-123` | This `expect` sits inside async connection setup. A reused attachment crashes the setup task and is hard to diagnose from the caller side. | Convert the missing receiver into a normal transport/setup error. |
| P2 | `crates/fireline-sandbox/src/lib.rs:271-277` | `unreachable!` is inside an async registration path. If control-flow assumptions break, the request panics instead of failing normally. | Replace with explicit error handling. |

## Category E — Resource leaks

No P1/P2 findings confirmed.

What I checked:
- `StateMaterializerTask`, `BootstrapHandle`, `StreamResourceRegistry`, and `StreamDeploymentPeerRegistry` task lifecycles
- shared terminal actor and child shutdown paths
- temporary file/directory cleanup sites in non-test code

What looked acceptable:
- background projection tasks that are stored and aborted on drop
- control-plane/bootstrap handles that retain spawned tasks and join/abort them during shutdown
- cleanup best-effort `remove_file` / `remove_dir_all` sites in tests only
