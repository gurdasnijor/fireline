# QA Review: `pi-acp -> OpenClaw` Demo Integration (2026-04-12)

Reviewed against current `origin/main` in an isolated worktree at `63f3461`. This was a step-by-step walkthrough of [docs/demos/pi-acp-to-openclaw.md](../demos/pi-acp-to-openclaw.md). No fixes were attempted in this pass.

## TL;DR

The demo is **not green on current main**.

What is real today:
- The TypeScript middleware surface from the north-star file exists: `trace()`, `approve()`, `budget()`, `secretsProxy()`, and `peer()` are implemented in `packages/client/src/middleware.ts`.
- Fireline can boot a local conductor + durable-streams stack from the current CLI/runtime path.
- The approval substrate is live: a prompt can emit a `permission_request` row to the durable stream, and an external `approval_resolved` append succeeds.
- `peer()` and `secretsProxy()` serialize into host topology components on the wire.

What breaks the advertised narrative:
- The literal CLI story in the demo doc is stale. Current CLI exposes `run` and `build`, not `deploy`, and the repo-local `fireline` bin is not available through `pnpm exec fireline`.
- `agent(['pi-acp'])` is not demo-usable on current main. Even after `fireline-agents add pi-acp`, the ACP socket closes during `initialize()`.
- `approve({ scope: 'tool_calls' })` still lowers to a prompt-level fallback gate, not true tool-call interception.
- Step 5 is blocked: `@fireline/state` does not currently build, and `src/bin/dashboard.rs` is still a stub.

## Step Matrix

| Step | Status | Evidence |
| --- | --- | --- |
| North-star file / API surface | `PARTIAL PASS` | The north-star file references `trace`, `approve`, `budget`, `secretsProxy`, and `peer` at `docs/demos/pi-acp-to-openclaw.md:61-68`. Those builders all exist in `packages/client/src/middleware.ts`. However, the same doc still says `secretsProxy()` “does not exist yet” at `docs/demos/pi-acp-to-openclaw.md:216-220`, which is now stale. |
| Step 1: “Run locally” as written | `FAIL AS WRITTEN` | Current CLI help exposes only `run` and `build`: `node packages/fireline/bin/fireline.js --help` prints those two commands only. `node packages/fireline/bin/fireline.js deploy agent.ts` fails with `fireline: unexpected argument: agent.ts`. Repo-local `fireline` is also not discoverable through `pnpm --dir packages/fireline exec fireline --help` (`Command "fireline" not found`). |
| Step 1: local Fireline substrate | `PASS` | A minimal spec booted successfully via `node packages/fireline/bin/fireline.js ...` once `fireline`, `fireline-streams`, and JS packages were built. The CLI printed `fireline ready`, a sandbox id, an ACP URL, and a durable state stream URL. This proves the local conductor/streams path exists on current main. |
| Step 1: local `agent(['pi-acp'])` demo | `FAIL` | Exact north-star style smoke with `agent(['pi-acp'])` printed a ready ACP URL, but `connectAcp(...).initialize()` immediately failed with `ACP connection closed`. Installing the agent first with `fireline-agents add pi-acp` succeeded, and `crates/fireline-tools/src/agent_catalog.rs:338-362` shows the single-token auto-install/resolve path exists, but the runtime still closed the ACP connection after startup. From a demo perspective this is a false-green boot. |
| Step 2: middleware stack exists locally | `PARTIAL PASS` | The middleware builders are real. In a live smoke with a stand-in ACP agent, the persisted `runtime_spec` row included `audit`, `approval_gate`, `budget`, `secrets_injection`, and `peer_mcp` components, which matches the advertised composition story. |
| Step 2: approval / pause / resume story | `PARTIAL PASS` | The durable approval substrate works: a prompt emitted a `permission_request` row on the state stream, and `appendApprovalResolved(...)` succeeded against that same stream. Strong supporting evidence also exists in CI: the `managed-agent-tests` job in run `24317388967` passed (`https://github.com/gurdasnijor/fireline/actions/runs/24317388967`). But the exact semantics are not what the demo claims yet: `approve({ scope: 'tool_calls' })` currently lowers to an always-match prompt gate in `packages/client/src/sandbox.ts:240-253`, with reason text `approval fallback: prompt-level gate until tool-call interception lands`. That means every prompt is gated, not only risky tool calls. |
| Step 3: `fireline build` / hosted image path | `BLOCKED` | `fireline build` exists, but the hosted-image path is not demo-green on current main. Local `fireline build` requires Docker; on this machine it failed because no daemon was available. More importantly, CI run `24317388967` failed its `docker-host-images` job because the Docker build context is missing `verification/audit/Cargo.toml`: `failed to load manifest for workspace member /app/verification/audit` and `failed to read /app/verification/audit/Cargo.toml`. |
| Step 3: `fireline deploy --to anthropic` | `BLOCKED ON MISSING CLI + PROVIDER WIRING` | There is no `deploy` verb in the current CLI. The doc’s `fireline deploy agent.ts --to anthropic` flow is therefore not runnable. I also tried the closest current substitute, `fireline run --provider anthropic ...`, and the resulting sandbox descriptor/runtime state still reported `provider: "local"`, so even the override path is not honestly exercising a remote Anthropic provider here. |
| Step 4: add a peer reviewer | `PARTIAL / BLOCKED` | The backend composition piece exists: `peer({ peers: [...] })` lowers to `peer_mcp` in `packages/client/src/sandbox.ts:276-281`, and the smoke stream showed peer tool descriptors being registered. But the demo’s actual operator flow depends on `fireline deploy reviewer.ts --to anthropic`, which does not exist today. So the peer substrate is partially there, while the documented end-to-end peer deployment story is blocked. |
| Step 5: OpenClaw-style control surface | `BLOCKED` | The doc’s productized control/observation story is not ready. `@fireline/state` currently fails to build: `packages/state/src/collections/pending-permissions.ts` and `session-permissions.ts` both reject `RequestId` as a key type in the current query config. `src/bin/dashboard.rs:17-37` is also still a TODO stub. The demo doc itself already partially admits this at `docs/demos/pi-acp-to-openclaw.md:221-222` (“A packaged control UI / message board does not exist yet”). |

## Shortest Path To Demo-Green

1. Make the local entrypoint honest.
   The literal “`npx fireline agent.ts`” story needs to match the actual CLI packaging/distribution path. Right now the repo only reliably works through `node packages/fireline/bin/fireline.js ...` plus prebuilt Rust binaries.

2. Make ACP agent startup fail closed instead of false-green.
   The biggest demo breaker is that `agent(['pi-acp'])` prints a ready ACP URL but the socket closes during `initialize()`. The demo cannot be declared healthy until either:
   - the installed `pi-acp` path actually stays up and answers ACP, or
   - Fireline reports startup failure instead of advertising a usable ACP endpoint.

3. Finish the approval semantics the demo claims.
   The current `approve({ scope: 'tool_calls' })` fallback is useful proof that pause/resume over durable streams works, but it is still prompt-level gating. Demo copy should not promise tool-call-only approval until tool-call interception lands.

4. Land a real remote deployment path.
   `fireline deploy --to ...` is the hinge for Steps 3 and 4. Without it, the “same file local -> Anthropic cloud -> peer composition” story is still a design doc, not a runnable demo.

5. Repair the hosted-image path in CI.
   Even after `deploy` exists, the Docker image path is not healthy while `docker-host-images` is red on current main due to the missing `verification/audit/Cargo.toml` in the Docker context.

6. Finish the Step 5 observation surface.
   Either `@fireline/state` must build cleanly again, or the demo must explicitly pivot to a narrower stream-inspection story. A TODO dashboard stub is not enough for an “OpenClaw-style” promise.

## Notes On What This Demo Already Proves

Even though the full walkthrough is not green, current main does already prove a meaningful subset of the story:

- Fireline can compose middleware around an ACP-facing agent definition in one TS file.
- The host persists that composition into a durable state stream.
- Approval can suspend a prompt, expose that pause as durable state, and accept an external resolution event.
- Peer and secret-proxy topology components exist at the client/host contract level.

That is real substrate. The gap is that the polished end-to-end demo narrative in `pi-acp-to-openclaw.md` still outruns the runnable product surface.

## Key Evidence

- Demo source: `docs/demos/pi-acp-to-openclaw.md`
- Middleware builders: `packages/client/src/middleware.ts`
- Approval fallback lowering: `packages/client/src/sandbox.ts:240-253`
- Peer lowering: `packages/client/src/sandbox.ts:276-281`
- Secrets injection lowering: `packages/client/src/sandbox.ts:292-303`
- Current CLI surface: `packages/fireline/src/cli.ts`
- Dashboard stub: `src/bin/dashboard.rs:17-37`
- ACP registry fallback install path: `crates/fireline-tools/src/agent_catalog.rs:338-362`
- Passing approval substrate CI: `managed-agent-tests` job in `https://github.com/gurdasnijor/fireline/actions/runs/24317388967`
- Failing hosted-image CI: `docker-host-images` job in the same run, failing on missing `/app/verification/audit/Cargo.toml`
