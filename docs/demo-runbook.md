# Fireline Demo Runbook — 2026-04-12

> Environment bring-up, port/process table, shutdown, and known-issues reference for the 2026-04-12 demo. Pair with [`./demo-walkthrough.md`](./demo-walkthrough.md) for the click-by-click script.

## 1. Pre-demo checklist (run ≥30 minutes before)

Execute in order. Treat each step as a hard gate — if one fails, **stop and investigate** before moving on.

1. **Working tree is clean and on `main` at the restructure-green commit.**
   ```sh
   cd ~/gurdasnijor/fireline
   git status                              # must be clean (no unstaged or untracked)
   git fetch origin
   git checkout main
   git pull --ff-only origin main
   git log -1 --format="%H %s"            # confirm HEAD matches the day's target commit
   ```
   > **TODO(demo-review):** name the "restructure-green" commit explicitly here once it lands. At dispatch time the workspace:13 restructure is in flight with `283a903` (session/sandbox move) and `a2b8227` (drop moved microsandbox source) as the latest commits — neither is yet CI-green. The pre-demo gate is "CI is green on whatever is at `origin/main` by 16:00 local on demo day."

2. **`cargo check --workspace` green.**
   ```sh
   cargo check --workspace
   ```
   Expected output: `Finished \`dev\` profile ... target(s) in <time>`. Zero errors, zero warnings you haven't seen before. **If this fails, do not proceed** — the restructure is mid-flight and a half-applied move will break the whole startup sequence. See §5 "Known issues — state 0: restructure not green".

3. **`pnpm install` succeeds.**
   ```sh
   pnpm install
   ```
   Should be a no-op on an up-to-date checkout. If packages change, re-run and verify the TS build in step 4.

4. **`@fireline/client` builds cleanly.**
   ```sh
   pnpm --filter @fireline/client build
   ```
   Expected output: `tsc` emits `dist/` with no errors. The `browser-harness` imports `@fireline/client/host`, `@fireline/client/host-fireline`, and `@fireline/state` — if any of them fail to build, the Vite dev server in step 2 below will refuse to start with an import resolution error.

5. **Confirm the required Rust binaries are present.**
   ```sh
   ls -la target/debug/fireline target/debug/fireline-control-plane target/debug/fireline-testy-load
   ```
   All three must exist and have a recent `mtime`. If any is missing, **do not run `pnpm --filter @fireline/browser-harness predev`** — the `predev` script invokes `cargo build` and would collide with the workspace:13 restructure lane. Instead, build the specific binaries by hand:
   ```sh
   cargo build -p fireline --bin fireline --bin fireline-testy-load -p fireline-control-plane --bin fireline-control-plane
   ```
   The `predev` equivalent (copied from `packages/browser-harness/package.json`) can be safely run **only** if the restructure is not in flight — which at dispatch time it is, so prefer the hand-build above.

6. **No dangling processes on demo ports.**
   ```sh
   lsof -i :4436 -i :4437 -i :4440 -i :5173
   ```
   Should return nothing. If anything is listed, run `pnpm --filter @fireline/browser-harness dev:clean` to kill stragglers, or manually `kill` the PIDs you see. See §3 below for what each port does.

7. **Smoke-check the control-plane health endpoint.** Start the control plane by hand (one-shot, kill it after) to confirm the binary itself is healthy before committing to the full startup sequence:
   ```sh
   target/debug/fireline-control-plane \
     --host 127.0.0.1 --port 4440 \
     --fireline-bin target/debug/fireline \
     --runtime-registry-path /tmp/fireline-precheck-runtimes.toml \
     --peer-directory-path /tmp/fireline-precheck-peers.toml &
   sleep 2
   curl -sS http://127.0.0.1:4440/healthz          # expect: ok
   kill %1
   wait 2>/dev/null
   ```
   If `curl` returns non-200 or the process exits before `sleep` finishes, do NOT attempt the demo — the binary is broken. Fall through to §5.

## 2. Startup sequence

Run in a single terminal. Keep it visible on your secondary monitor during the demo so you can see the control-plane logs.

```sh
cd ~/gurdasnijor/fireline

# Option A (preferred — works when target/debug binaries are already built):
pnpm --filter @fireline/browser-harness dev
```

What `pnpm dev` actually does, per `packages/browser-harness/package.json:10`:

```
concurrently -k --kill-signal SIGTERM \
  -n control,vite -c cyan,magenta \
  "pnpm dev:server" "pnpm dev:vite"
```

- `pnpm dev:server` runs `node ./dev-server.mjs`, which:
  - Creates `.tmp/runtimes.toml` and `.tmp/peers.toml` under `packages/browser-harness/.tmp/`.
  - Spawns `target/debug/fireline-control-plane` on `127.0.0.1:4440` (see `dev-server.mjs:205-244`).
  - Serves the browser-harness control-server API on `http://127.0.0.1:4436` (agents list, resolve, runtime CRUD).
  - Blocks on `waitForHttpReady` polling `http://127.0.0.1:4440/healthz` before accepting browser requests.
- `pnpm dev:vite` runs `vite --host --strictPort`, which:
  - Serves the React app on `http://localhost:5173`.
  - Proxies `/api → :4436`, `/cp → :4440` (with `/cp` prefix stripped), `/acp → :4437` (WebSocket), `/v1 → :4437`, `/healthz → :4437` per `vite.config.ts`.

**Option B** (if the `predev` script was skipped or the binaries need a rebuild, and the restructure is NOT in flight):

```sh
pnpm --filter @fireline/browser-harness predev    # ⚠️ invokes cargo build; collides with workspace:13
pnpm --filter @fireline/browser-harness dev
```

At dispatch time **use Option A exclusively** — the predev's embedded `cargo build` will fight the workspace:13 restructure lane.

**Expected startup log, in order:**

1. `starting fireline-control-plane with prefer_push=false`
2. `[control-plane] ...` startup lines from the Rust binary (binding, registry path, etc.)
3. `browser harness control server ready on http://127.0.0.1:4436`
4. `VITE v6.x.x  ready in <ms>` (from vite's side)
5. `➜  Local:   http://localhost:5173/`

**Navigate to `http://localhost:5173`** in a fresh browser tab (Chrome preferred for the demo — Vite's HMR has the best React devtools integration there). You should see the *"Fireline Browser Harness"* header and the split-pane layout with the runtime controls on the left and the State Explorer on the right. **Do not click anything yet** — the walkthrough's first step is in `demo-walkthrough.md` §2.1.

## 3. Port and process table

| Port | Process | What it serves | Started by |
|---|---|---|---|
| `4436` | `node ./dev-server.mjs` | Browser-harness control server — `/api/agents`, `/api/resolve`, `/api/runtime` CRUD. Backs the left-pane agent dropdown and legacy runtime lifecycle fallback. | `pnpm dev:server` |
| `4437` | `target/debug/fireline` | Fireline runtime process — exposes `/acp` (ACP WebSocket), `/v1/stream/*` (durable-streams-server embedded via `stream_host.rs`), `/healthz`, and `/fs/*` helper routes. This is the runtime the control plane spawns on demand when the browser calls `host.provision(...)`. | `fireline-control-plane` (spawned as a child process by the control plane, NOT by the dev server directly) |
| `4440` | `target/debug/fireline-control-plane` | Control plane HTTP API — `POST /v1/runtimes`, `GET /v1/runtimes/{key}`, `POST /v1/runtimes/{key}/stop`, `DELETE /v1/runtimes/{key}`, `GET /healthz`. The `Host` satisfier in the browser talks to this through the vite `/cp` proxy. | `dev-server.mjs` (line 210) |
| `5173` | `vite` | React dev server + HMR for `packages/browser-harness/src/`. Also hosts the proxy table that routes `/api`, `/cp`, `/acp`, `/v1`, and `/healthz` to the ports above. | `pnpm dev:vite` |

Debugging commands:

```sh
# Is anything listening on a given port?
lsof -i :4436                                     # dev-server API
lsof -i :4437                                     # fireline runtime
lsof -i :4440                                     # control plane
lsof -i :5173                                     # vite

# All four at once, with process names and PIDs:
lsof -i :4436 -i :4437 -i :4440 -i :5173 -P -sTCP:LISTEN

# Process tree starting from the dev-server parent:
pgrep -la fireline                                # every fireline-* binary currently running
pgrep -fa "node ./dev-server.mjs"                 # dev-server specifically
pgrep -fa vite                                    # vite

# Direct health probes (bypass vite proxy):
curl -sS http://127.0.0.1:4440/healthz            # control plane
curl -sS http://127.0.0.1:4437/healthz            # fireline runtime (only works if a runtime is live)
```

**Why port 4437 is pinned:** the vite proxy config hardcodes `/acp`, `/v1`, and `/healthz` to `http://127.0.0.1:4437` (see `packages/browser-harness/vite.config.ts:16-25`). The browser has no dynamic routing — whatever runtime answers on 4437 is *the* runtime for the demo. The Tier 5 `createFirelineHost.provision` path actually POSTs `port: 0` to the control plane (see the Tier 5 smoke test at `packages/browser-harness/test/tier5-smoke.browser.test.ts:79-88`), so the spawned runtime gets an OS-assigned ephemeral port — and the demo works because in a clean dev environment 4437 happens to be the port the child binds to. If that assumption breaks (another process squats on 4437 before the runtime binds, or the control plane's port-assignment behavior changes), the ACP WebSocket will fail to connect even though the runtime itself is healthy. See §5c "state explorer never populates" for the diagnosis ladder. The legacy `dev-server.mjs:121 POST /api/runtime` path does hardcode `port: 4437`, but that handler is **no longer called by the Tier 5 browser-harness** — it's still wired for backward compatibility only.

**Why port 4436 is the dev-server API:** the dev-server.mjs was written before the `Host` primitive existed and originally fronted the entire runtime lifecycle. After Tier 5 (`52c31af`), the browser creates runtimes directly through the control plane via `createFirelineHost`, and the 4436 API is mostly reduced to the **agent catalog endpoints** (`/api/agents` and `/api/resolve?agentId=...`). The legacy `/api/runtime` CRUD is still wired, but the browser-harness app doesn't use it post-Tier 5 — see `packages/browser-harness/src/app.tsx:434-458` where `launchRuntime()` calls `host.provision(...)` directly (the verb was renamed from `createSession` in commit `37db346`).

## 4. Shutdown

1. **Return focus to the terminal running `pnpm dev`** and press **Ctrl+C** once. `concurrently` receives the interrupt and forwards SIGTERM to both child processes (control plane via dev-server, vite directly). Give it up to 5 seconds to unwind.

2. **Run the clean script** to remove any `.tmp/` state and kill any stragglers that didn't respond to SIGTERM:
   ```sh
   pnpm --filter @fireline/browser-harness dev:clean
   ```
   What this does, per `package.json:13`:
   ```
   rm -rf ./.tmp && \
     for p in 4437 4440 5173; do lsof -ti tcp:$p 2>/dev/null | xargs kill 2>/dev/null; done; true
   ```
   Note: `dev:clean` does **not** kill port 4436 (the dev-server itself). If you need to force-kill that too:
   ```sh
   lsof -ti tcp:4436 | xargs kill
   ```

3. **Verify no dangling ports.**
   ```sh
   lsof -i :4436 -i :4437 -i :4440 -i :5173
   ```
   Should return nothing. If a port is still bound, find the PID and kill it explicitly.

4. **If you plan to demo again immediately after shutdown**, also rotate the `.tmp` state:
   ```sh
   rm -rf packages/browser-harness/.tmp
   ```
   This removes `runtimes.toml` and `peers.toml` so the next startup sees a clean slate. The control plane will recreate both on launch.

## 5. Known issues and fallback recipes

### State 0 — restructure not green

**Symptom:** `cargo check --workspace` fails in the pre-demo checklist (step 1.2). Typical error is an import path mismatch like `unresolved import 'fireline_conductor::runtime::mounter'` or a dangling re-export in `fireline-components/src/lib.rs`.

**Cause:** the workspace:13 crate-restructure lane is mid-sequence. Commits are actively moving files between `fireline-conductor`/`fireline-components` and the new primitive-aligned crates (`fireline-session`, `fireline-orchestration`, `fireline-harness`, `fireline-sandbox`, `fireline-resources`, `fireline-tools`, `fireline-runtime`), and some intermediate commits leave the workspace temporarily un-compilable.

**Recovery:**
1. `git log --oneline -10 origin/main` — find the most recent commit whose subject does not mention "Move", "Drop", "Register", or "primitive skeletons". That's the last restructure-stable point.
2. Check out that commit explicitly: `git checkout <sha>`. **Do not stay on `main` for the demo** if `main`'s tip is mid-restructure.
3. Re-run `cargo check --workspace` on the pinned sha. If it's green, proceed. If it isn't, escalate to the workspace:13 lead before demo time.
4. **After the demo**, return to `main`: `git checkout main`.

**Preferred outcome:** the restructure lands green on `main` before the demo and this recovery is unnecessary. Worth re-checking `origin/main` CI status at T-1 hour.

### 5a — control plane refuses to start

**Symptom:** startup sequence fails with `timed out waiting for control plane to become ready` from `dev-server.mjs` after ~10 seconds, or the browser loads but clicking **"Launch Agent"** immediately produces `Failed to fetch` / `HTTP 502` against `/cp/v1/runtimes`.

**Diagnosis:**
```sh
# Was the binary actually spawned?
pgrep -fa fireline-control-plane

# Is anything bound to 4440?
lsof -i :4440

# Direct health probe (bypasses vite):
curl -sS http://127.0.0.1:4440/healthz

# If the dev-server is running, its stdout prefixed with `[control-plane]` will
# have captured the child's stdout+stderr. Look in the terminal you launched
# `pnpm dev` from for any Rust panic or bind failure.
```

**Recovery — Option A (most common, port squatter):** another process is already bound to 4440. Common cause: an earlier `pnpm dev` session didn't fully clean up. Run:
```sh
pnpm --filter @fireline/browser-harness dev:clean   # kills 4437, 4440, 5173
lsof -ti tcp:4440 | xargs kill 2>/dev/null; true    # belt-and-suspenders
```
Then re-run `pnpm --filter @fireline/browser-harness dev`.

**Recovery — Option B (hand-start the control plane):** if the dev-server's auto-spawn path is broken for any reason, start the control plane in a separate terminal using the exact invocation from `dev-server.mjs:210-227`:

```sh
cd ~/gurdasnijor/fireline
mkdir -p packages/browser-harness/.tmp
target/debug/fireline-control-plane \
  --host 127.0.0.1 \
  --port 4440 \
  --fireline-bin target/debug/fireline \
  --runtime-registry-path packages/browser-harness/.tmp/runtimes.toml \
  --peer-directory-path packages/browser-harness/.tmp/peers.toml \
  --startup-timeout-ms 20000 \
  --stop-timeout-ms 10000
```

Leave that running. Then in another terminal, start vite only:
```sh
pnpm --filter @fireline/browser-harness exec vite --host --strictPort
```
And the dev-server's API separately (for the agent catalog):
```sh
pnpm --filter @fireline/browser-harness exec node ./dev-server.mjs
```
The dev-server will detect port 4440 is already in use and skip its spawn. **TODO(demo-review):** verify this fallback actually works — `dev-server.mjs:205-244` doesn't have explicit "detect-existing" logic; it unconditionally spawns. Pre-demo test on Saturday morning.

**Recovery — Option C (binary broken):** if `curl http://127.0.0.1:4440/healthz` returns non-200 after you've verified the process is running, or the binary panics at startup, the control plane itself is broken. Symptoms from workspace:13's mid-restructure commits — fall back to an earlier commit per "state 0" above.

### 5b — runtime boots, prompts 404 or error

**Symptom:** **"Launch Agent"** succeeds. **"New Session"** succeeds (real `sessionId` in the event log and inspector). Typing a prompt and clicking **"Send"** produces a `prompt_response` event with an `error` payload, a `session_update` that's not a text content block, or a `Failed to fetch` error in the browser console.

**Diagnosis:**
```sh
# Is the runtime process alive?
pgrep -fa "target/debug/fireline "              # trailing space to exclude fireline-control-plane

# Direct ACP health probe:
curl -sS http://127.0.0.1:4437/healthz

# What agent command is the catalog resolving?
curl -sS "http://127.0.0.1:4436/api/resolve?agentId=fireline-testy-load"
```

**Cause most likely:**
- `fireline-testy-load` binary is stale — built against an older ACP SDK than `fireline` expects. Rebuild it: `cargo build -p fireline --bin fireline-testy-load`.
- The ACP WebSocket is being proxied incorrectly — check `vite.config.ts` hasn't been edited and `/acp` still points at `ws://127.0.0.1:4437/acp`.
- The testy-load agent process crashed inside the runtime. The runtime's own stdout (visible in the terminal running `pnpm dev` under the fireline runtime's prefixed output, if any) will show the panic.

**Recovery:**
1. Click **"Stop Runtime"** in the UI.
2. Click **"Reset"** (clears event log and disconnect).
3. Click **"Launch Agent"** → **"New Session"** → send prompt again.
4. If it's still broken, kill the entire dev session with Ctrl+C, rebuild testy-load, and restart:
   ```sh
   cargo build -p fireline --bin fireline-testy-load
   pnpm --filter @fireline/browser-harness dev
   ```

**Narrative fallback during demo:** if prompts fail but the runtime + session flow is green, you can still demo the `WakeOnReadyIsNoop` beat (click Wake → noop) and the state-explorer panel — both work even when the agent itself is broken, because they rely on Host / Session primitives, not on Harness success.

### 5c — state explorer never populates

**Symptom:** after **"Launch Agent"** and **"New Session"**, the right-pane State Explorer shows *"Connecting durable state…"* indefinitely, OR *"State stream error: ..."* with a message mentioning the stream URL.

**Diagnosis:**
```sh
# Is the state stream endpoint responding?
curl -sS "http://127.0.0.1:4437/v1/stream/fireline-harness-state?offset=-1&live=false" | head

# Is the vite proxy routing /v1 correctly?
curl -sS "http://localhost:5173/v1/stream/fireline-harness-state?offset=-1&live=false" | head
```

If the direct probe works but the vite-proxied one doesn't, the vite config got out of sync. Check `packages/browser-harness/vite.config.ts:20-23` for the `/v1` proxy target.

**Cause most likely:**
- The runtime hasn't written anything to the stream yet — this is normal for the first ~100ms after runtime launch. **Wait 2–3 seconds and try again.**
- The `fireline-harness-state` stream name is mismatched. The browser uses `VITE_FIRELINE_STATE_STREAM` (defaulting to `fireline-harness-state` per `app.tsx:24`), and the runtime is started by `dev-server.mjs:123` with `stateStream: 'fireline-harness-state'`. If either has been edited, they need to match.
- `@fireline/state`'s `createFirelineDB.preload()` threw. Browser console will have the error.

**Recovery:**
1. Browser console → look for network errors against `/v1/stream/fireline-harness-state`.
2. If the stream 404s, stop and restart the runtime via **"Stop Runtime"** → **"Launch Agent"**.
3. If the state explorer error message mentions a schema mismatch, the `@fireline/state` schema is out of sync with the runtime's emitted envelopes. Rebuild: `pnpm --filter @fireline/state build` (if it exists) or just `pnpm --filter @fireline/client build` (which transitively handles state).

**Narrative fallback during demo:** pivot to the left-pane **inspector card** ("Current Session"), which is React-state-backed and works even when the state explorer is broken. Walk through `status / sessionId / sessionStatus / lastError / handleId` and tell the audience: *"the inspector here is the minimum read surface any Host satisfier exposes; the State Explorer on the right is a richer view backed by `@fireline/state` live queries — and here it's degraded, but notice the rest of the substrate is still green."* The walkthrough explicitly anticipates this in §5c.

### 5d — browser-harness TypeScript hot-reload stalls

**Symptom:** Vite's HMR stops propagating changes (usually indicated by a "page is out of date" warning in the console) or the React app crashes with a stale import.

**Cause most likely:** Vite's HMR loses track of the app when you rapidly kill+restart the control plane — the WebSocket to the runtime breaks, and the app's useEffect cleanups get stuck.

**Recovery:** full browser-tab reload (Cmd+R / Ctrl+R). HMR re-establishes against the vite dev server on 5173.

### 5e — browser can't connect to localhost (rare)

**Symptom:** `http://localhost:5173` doesn't load, or hangs indefinitely.

**Cause most likely:** `/etc/hosts` or a VPN intercepting `localhost`. Run `ping localhost` → should resolve to `127.0.0.1`. If it doesn't, fix `/etc/hosts` or disable the VPN.

**Recovery:** replace `localhost` with `127.0.0.1` in the browser URL — `http://127.0.0.1:5173`. Vite's `--host` flag binds to all interfaces, so this works.

---

## 6. Pre-demo dry run checklist (T-30 minutes)

Do one full dry run before going live. Walk through the demo script end-to-end on your actual demo hardware.

- [ ] Fresh `cargo check --workspace` green
- [ ] Fresh `pnpm install` + `pnpm --filter @fireline/client build` green
- [ ] `pnpm --filter @fireline/browser-harness dev` starts cleanly
- [ ] Browser at `http://localhost:5173` shows the harness header
- [ ] Click through §2.1 (Launch Agent) — see `runtime_launch` event, `sessionStatus: running`, State Explorer activates
- [ ] Click through §2.2 (New Session) — see `session_new` event, `sessionId` populates, new row in `sessions` tab
- [ ] Click through §2.3 (Send prompt) — see `Hello, world!` response, new row in `turns` tab
- [ ] Click through §2.4 (Disconnect + Reconnect + Load) — `session_load` event, rows preserved
- [ ] Click through §3 (Wake) — `{ kind: 'noop' }` event, nothing else changes
- [ ] Click through §4 (State Explorer tabs) — all five tabs render without error
- [ ] Clean shutdown (Ctrl+C + `dev:clean`) leaves no dangling ports

If any checkbox fails, fix it **before** the audience is in the room. If a checkbox is timing out for a reason you can't diagnose in 5 minutes, drop to the narrative fallback in `demo-walkthrough.md` §5 for that beat.

---

## TODO(demo-review) items captured inline

1. **§1.1 restructure-green commit identity** — name the exact sha the demo runs against once the workspace:13 restructure lands green. Update at T-1 hour.
2. **§5a Option B hand-start fallback** — verify `dev-server.mjs`'s auto-spawn path tolerates a pre-existing control-plane on 4440. If it doesn't (it probably doesn't — `dev-server.mjs:205-244` unconditionally spawns), the fallback needs a custom dev-server start that skips the spawn. Pre-demo test Saturday morning.
3. **§1.5 binary presence check** — if the workspace:13 restructure renames binary targets, update the `ls target/debug/fireline*` and the manual `cargo build -p fireline --bin fireline --bin fireline-testy-load` commands accordingly. They reference the pre-restructure crate names.
4. **§6 dry run** — actually run this list end-to-end on demo hardware ≥30 minutes before the demo. Check off each item on paper.
