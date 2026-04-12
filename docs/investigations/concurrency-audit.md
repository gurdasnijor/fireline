# Concurrency audit

## TL;DR

- `RuntimeRegistry` and the `RuntimeHost` state transitions built on top of it are **not atomic today**. The registry is a file-backed read/modify/write store with no in-process mutex, no file lock, and a shared temp-file name. Concurrent `provision` / `register` / `heartbeat` / stale-scan writes can lose updates or overwrite each other.
- The clearest lock-scope bug is in `MicrosandboxSandbox::execute`, which holds the `live` mutex across guest I/O `await`s.
- The clearest async-lifecycle bug is in `StateMaterializerTask::preload()`: if the background materializer dies before the first `up_to_date`, `preload()` waits forever.
- I did **not** find any `unsafe impl Send`, `unsafe impl Sync`, `spawn_local`, or `Rc<...>`/`RefCell<...>` async workarounds in `crates/` or `src/`.
- I did **not** find any `tokio::sync::mpsc::unbounded_*` usage. The unbounded-growth risks are ordinary `HashMap` caches and waiter registries.

## Method

- Searched lock sites with `rg -n 'Mutex::new|RwLock::new|lock\\(|write\\(|read\\(' crates/ src/ --glob '*.rs'`.
- Searched growth sites with `rg -n 'Vec::new|HashMap::new|tokio::sync::mpsc::unbounded|unbounded_channel|broadcast::channel|mpsc::channel\\(' crates/ src/ --glob '*.rs'`.
- Searched task creation with `rg -n 'tokio::spawn|spawn_local|JoinHandle|abort\\(' crates/ src/ --glob '*.rs'`.
- Read the high-risk files manually to separate real correctness bugs from benign lock usage.

## Findings

### P1 — `RuntimeRegistry` is a non-atomic file-backed read/modify/write store

- `crates/fireline-sandbox/src/registry.rs:44-49`
- `crates/fireline-sandbox/src/registry.rs:59-67`
- `crates/fireline-sandbox/src/registry.rs:109-114`
- `crates/fireline-sandbox/src/lib.rs:86-98`
- `crates/fireline-sandbox/src/lib.rs:220-227`
- `crates/fireline-sandbox/src/lib.rs:307-345`
- `crates/fireline-host/src/control_plane.rs:136-189`

Issue:
`RuntimeRegistry` reads the whole TOML file, mutates an in-memory vector, then writes the whole file back with no mutex or file lock. Every caller that does `get`/decide/`upsert` or `get`/decide/`remove` is therefore racing with every other caller. The temp path is also only `path.with_extension(format!("tmp-{}", std::process::id()))`, so two concurrent writers in the same process share the same temp filename.

Why this is a bug:
two overlapping writes can lose status transitions, resurrect stale descriptors, or fail spuriously during rename. Concrete races include:

- `stop()` reading a descriptor and writing `Stopped` while the stale-runtime scanner reads the old `Ready` value and writes back `Stale`.
- `heartbeat()` refreshing a descriptor while `register()` or `stop()` is writing a different version.
- concurrent `upsert()` calls writing through the same `*.tmp-<pid>` path.

Fix:
move the registry behind a single synchronization boundary. At minimum:

- add an in-process async mutex around every file-backed read/modify/write,
- use a unique temp file per write (`uuid`, `mkstemp`, or `tempfile`),
- add a real file lock if multiple processes can touch the same registry file,
- collapse `get`/decide/`upsert` into atomic transition methods on the registry rather than exposing the raw pieces.

### P1 — `MicrosandboxSandbox::execute` holds the `live` mutex across guest I/O

- `crates/fireline-sandbox/src/microsandbox.rs:194-215`
- `crates/fireline-sandbox/src/microsandbox.rs:224-255`

Issue:
`execute()` acquires `self.live.lock().await`, takes a reference to the sandbox from the guard, and then awaits `sandbox.shell(...)` / `sandbox.exec(...)` while the guard is still live.

Why this is a bug:
that serializes every tool call for every live microsandbox behind one mutex and can block `release()` behind a long-running guest command. It is exactly the async anti-pattern “hold a mutex guard across `.await`”.

Fix:
store `Arc<MsbSandbox>` in the map and clone the `Arc` out before awaiting, or remove the handle from the map temporarily, or split the map lookup from execution so the mutex is dropped before guest I/O starts.

### P1 — session-scoped secret resolution has a check-then-await race

- `crates/fireline-harness/src/secrets.rs:46-74`

Issue:
`pre_resolve_session_env()` checks `cached_session_env(session_id)` under a mutex, releases the lock, resolves every secret with `await`, then reacquires the mutex and inserts. Two concurrent prompts for the same session can both miss the cache and both hit the external resolver.

Why this is a bug:
this breaks the intended “session scope resolves once” semantics and can stampede the credential backend. Because the inserts use `or_insert`, one result wins, but the duplicated I/O and side effects have already happened.

Fix:
use a per-session in-flight state (`OnceCell`, `Shared<BoxFuture<_>>`, or a per-session async mutex) so only one resolver task does the work and the others await the same result.

### P1 — `StateMaterializerTask::preload()` can block forever if the worker exits early

- `crates/fireline-session/src/state_materializer.rs:196-200`
- `crates/fireline-session/src/state_materializer.rs:218-228`
- `crates/fireline-session/src/state_materializer.rs:243-247`

Issue:
`preload()` waits until `is_up_to_date == true`. If `consume_state_stream()` returns before setting that flag, `preload()` has no error path and no completion signal to observe.

Why this is a bug:
callers like `materialize_session_index()` can hang forever on startup if the stream reader fails to build or exits with a non-retryable error before the first `up_to_date`.

Fix:
track terminal task completion explicitly. For example:

- add a `oneshot::Sender<Result<()>>` or `watch` state for `Ready | Failed | Closed`,
- notify waiters on both success and failure,
- have `preload()` return an error when the background task exits before readiness.

### P1 — `StreamResourceRegistry` dies permanently on the first stream error

- `crates/fireline-resources/src/registry.rs:127-145`

Issue:
the resource projection task returns immediately on any `build()` error or any `reader.next_chunk().await` error. There is no retry loop, no backoff, and no signal to subscribers that the projection task has died.

Why this is a bug:
a transient durable-streams hiccup permanently freezes resource discovery for the lifetime of the process. Because the task is only stored as a `JoinHandle<()>`, the failure is silent unless a caller happens to inspect behavior later.

Fix:
mirror the retry structure already used by `StreamDeploymentPeerRegistry`: log the error, sleep/back off, rebuild the reader, and only terminate on explicitly non-retryable conditions. Also expose a health/error surface so callers can distinguish “empty index” from “dead projection task”.

### P1 — `ResourceRegistry::subscribe()` has a snapshot-to-subscription race

- `crates/fireline-resources/src/registry.rs:103-113`

Issue:
`subscribe()` sends the current snapshot to the watcher before calling `self.updates.subscribe()`.

Why this is a bug:
any update that lands between the initial `list()` snapshot and the later `subscribe()` call is lost forever for that watcher. This is a classic snapshot-then-subscribe race.

Fix:
subscribe first, then deliver a versioned snapshot, or switch to a `watch`/versioned state model where the watcher can observe “current state plus all later deltas” without a gap.

### P2 — resource subscribers can silently drop out on `broadcast` lag

- `crates/fireline-resources/src/registry.rs:106-113`

Issue:
the subscriber task uses `while let Ok(entries) = receiver.recv().await`. `tokio::sync::broadcast::Receiver::recv()` returns `Err(Lagged(_))` when a slow watcher falls behind the bounded channel, and this loop exits immediately with no log and no recovery.

Why this is a bug:
a slow or briefly stalled watcher stops receiving updates forever, silently.

Fix:
handle `RecvError::Lagged(_)` explicitly: log it, rebuild from `self.list().await?`, and continue. Only terminate on `Closed` or on an explicit watcher error.

### P2 — `ActiveTurnIndex` leaks waiter entries indefinitely

- `crates/fireline-session/src/active_turn_index.rs:47-62`
- `crates/fireline-session/src/active_turn_index.rs:87-89`
- `crates/fireline-session/src/active_turn_index.rs:117-120`

Issue:
`wait_for()` inserts an `Arc<Notify>` into `waiters` per `session_id`, but successful wakeups and timeouts do not remove that entry. The map is only cleared on full projection reset.

Why this is a bug:
over time, a long-lived runtime accumulates one `Notify` per ever-waited session id. This is bounded only by total historical session count, not active sessions.

Fix:
remove waiter entries after a successful notification or timeout, or replace the side map with a per-call `Notify`/`watch` primitive that does not require global retention.

### P2 — `DockerProvider::ensure_image_ready()` holds its mutex across slow Docker I/O

- `crates/fireline-sandbox/src/providers/docker.rs:66-100`

Issue:
the `image_ready` mutex is acquired before `inspect_image().await`, held during tar creation, and held through the entire streaming Docker build loop.

Why this is a bug:
it serializes every concurrent runtime start behind one long-lived mutex guard. If the image pull/build takes minutes, unrelated callers block on the mutex instead of awaiting a shared one-time initialization primitive.

Fix:
use `OnceCell`, `tokio::sync::OnceCell`, or an initialization future shared across callers. If a mutex remains, keep it only long enough to observe/update state, not across Docker network/build I/O.

### P2 — secret caches retain sensitive data without any eviction path

- `crates/fireline-harness/src/secrets.rs:25-29`
- `crates/fireline-harness/src/secrets.rs:66-87`

Issue:
`session_cache` and `once_cache` are process-lifetime `HashMap`s with no teardown, TTL, or session-end eviction. `once_cache` is especially suspicious because it is allocated but currently unused, so if it is wired later it will inherit the same problem.

Why this is a bug:
it is both an unbounded-growth risk and a sensitive-data-retention risk. Session-scoped secrets stay resident for the whole process lifetime unless the process exits.

Fix:
add explicit cache invalidation on session end, TTL-based eviction, and zeroizing removal paths. If `once_cache` is not needed yet, remove it until a real lifecycle exists.

### P2 — the stale-runtime scanner is fire-and-forget and untracked

- `crates/fireline-host/src/control_plane.rs:106-111`
- `crates/fireline-host/src/control_plane.rs:130-189`

Issue:
`spawn_stale_runtime_task()` creates a detached task and drops the `JoinHandle`.

Why this is a bug:
there is no structured shutdown, no panic observation, and no way for the control plane to report that the stale-scan loop died. The process keeps serving even if the background liveness maintenance task is gone.

Fix:
store the `JoinHandle` in a control-plane handle or task group, abort/join it on shutdown, and surface task failure in logs or health reporting.

## Category E check

I did not find:

- `unsafe impl Send`
- `unsafe impl Sync`
- `spawn_local`
- `Rc<...>` / `RefCell<...>` async ownership workarounds

So there is no evidence of manual `Send`/`Sync` escape hatches. The problems above are ordinary async coordination bugs, not trait-system bypasses.
