# Fireline CLI Production-Readiness Gap Analysis + Design

> Status: gap-analysis + follow-on design
> Date: 2026-04-12
> Scope: `packages/fireline/` production-readiness work after the shipped `run` command

This is a small gap document, not a full subsystem execution plan. The base CLI already exists in `packages/fireline/`. The remaining work is to close the production gap: remote deploy, packaging, interactive REPL, and always-on deployment wiring.

The hosted target for `fireline deploy` is described by [hosted-fireline-deployment.md](./hosted-fireline-deployment.md). This doc focuses on the CLI-side closing work needed to target that hosted instance cleanly.

## 1. Current State

`packages/fireline/` already ships a usable local CLI:

- `packages/fireline/src/cli.ts` implements `fireline run <file>` and implicit `fireline <file>`.
- It parses `--port`, `--streams-port`, `--state-stream`, `--name`, `--provider`, `--repl`, and `--help`.
- It loads the spec file via `tsx` `tsImport`, requires a default export, and verifies the export has `.start()`.
- It resolves the Rust binaries through `resolve-binary.ts` in this order:
  1. `FIRELINE_BIN` / `FIRELINE_STREAMS_BIN`
  2. platform package lookup (`@fireline/cli-darwin-arm64`, etc.)
  3. `target/debug` / `target/release` fallback
- It starts `fireline-streams`, then `fireline --control-plane`, waits for both `/healthz` endpoints, and calls `spec.start({ serverUrl, stateStream, name })`.
- It prints the sandbox id, ACP URL, and state URL, waits for `SIGINT` / `SIGTERM`, then destroys the sandbox and tears down both child processes.
- `packages/fireline/README.md` and [docs/guide/cli.md](../guide/cli.md) document this local flow accurately.

What is notably not shipped:

- no `deploy` subcommand
- no published platform binary packages
- `--repl` is a stub that only prints a message
- no hosted-instance discovery or auth story
- no always-on deployment wiring
- no real automated CLI tests yet; `packages/fireline/package.json` still has `"test": "echo 'no tests yet'"` and only carries a `test-fixtures/minimal-spec.ts` helper

## 2. Gap

### 2.1 `fireline deploy <file>`

The CLI can start a local control plane, but it cannot push a spec to a hosted Fireline instance. That blocks the core production path: author locally, deploy remotely, reconnect later.

### 2.2 Platform binary packaging

`resolve-binary.ts` already knows the desired packaging scheme, but the platform packages are still lookup stubs. Today `npx fireline` works only against a locally built Rust workspace.

### 2.3 `--repl`

The flag exists, but it does not connect to ACP or provide an interactive prompt loop. The current behavior is only "print the ACP URL and wait."

### 2.4 Always-on deployment wiring

The CLI has no way to tell a hosted instance that a deployment should stay warm. Per [durable-subscriber-execution.md](./durable-subscriber-execution.md), this should lower to `AlwaysOnDeploymentSubscriber`, not invent a second lifecycle mechanism.

### 2.5 Hosted-instance discovery and auth

There is no deploy target resolution, config file, or token handling. A remote deploy command needs a concrete answer to:

- which Fireline host receives the spec?
- how does the CLI authenticate?
- where does a team encode staging vs production?

## 3. Proposed Design

### 3.1 `deploy` command

Add a new subcommand:

```bash
fireline deploy <file.ts> --remote <url> [--name <name>] [--provider <provider>] [--always-on]
```

Behavior:

1. Load the default-exported Harness exactly like `run`.
2. Serialize the spec plus deploy metadata.
3. Send it to the hosted instance over HTTP.
4. Print the returned deployment id, ACP endpoint, and state endpoint.
5. If `--repl` is set, connect to ACP and open an interactive loop.

MVP request shape:

```json
{
  "name": "reviewer",
  "spec": { "...": "serialized Harness" },
  "providerOverride": "docker",
  "lifecycle": { "alwaysOn": false }
}
```

MVP endpoint:

```text
PUT /v1/deployments/{name}
Authorization: Bearer <token>
```

This is intentionally narrow. The exact hosted API can align later with the dedicated hosted-deployment proposal, but the CLI needs one concrete contract to target.

### 3.2 Hosted target resolution and auth

Phase 1 should require explicit `--remote`. Phase 4 adds project config.

Proposed resolution order once Phase 4 lands:

1. `--remote <url>`
2. `--target <name>` from `fireline.config.ts`
3. `defaultTarget` from `fireline.config.ts`
4. error

Proposed auth resolution order:

1. `--token <token>`
2. target config `auth.tokenFromEnv`
3. `FIRELINE_TOKEN`
4. unauthenticated request only for localhost / explicitly insecure targets

Minimal config shape:

```ts
export default {
  defaultTarget: 'production',
  targets: {
    production: {
      host: 'https://agents.example.com',
      auth: { tokenFromEnv: 'FIRELINE_TOKEN' },
    },
  },
}
```

This keeps the CLI aligned with [deployment-and-remote-handoff.md](./deployment-and-remote-handoff.md): config owns environment selection; flags are overrides and diagnostics.

### 3.3 Platform binary packaging

Keep the existing `resolve-binary.ts` lookup order, but make step 2 real:

- publish `@fireline/cli-darwin-arm64`
- publish `@fireline/cli-darwin-x64`
- publish `@fireline/cli-linux-arm64`
- publish `@fireline/cli-linux-x64`
- publish `@fireline/cli-win32-x64`

Each optional package should contain:

- `bin/fireline`
- `bin/fireline-streams`
- a tiny `package.json`

`@fireline/cli` then lists them as `optionalDependencies`, matching the esbuild/Turbo pattern already assumed by `resolve-binary.ts`.

### 3.4 `--repl`

`--repl` should become a real ACP shell, not a placeholder.

Proposed behavior:

- after `run` or `deploy`, call `agent.connect('fireline-cli')`
- read prompt lines from stdin
- send them over ACP as prompt requests
- stream assistant text to stdout
- `Ctrl+C` exits the REPL and then runs the normal teardown path

This should stay intentionally small. It is a debug/operator convenience, not a full TUI.

### 3.5 `--always-on`

Add `--always-on` to `deploy`, but define it as a deploy-time policy bit, not client-side lifecycle logic.

Behavior:

- CLI includes `lifecycle.alwaysOn = true` in the deploy request
- hosted Fireline persists that desired policy with the deployment
- hosted Fireline lowers the policy to `AlwaysOnDeploymentSubscriber`
- boot-time scan / heartbeat emits `deployment_wake_requested`
- the subscriber drives the existing wake/provision path until `sandbox_provisioned`

The CLI does not poll, retry, or supervise the deployment itself. It only declares the desired policy.

## 4. Phased Rollout

### Phase 1: `fireline deploy` MVP

Scope:

- add `deploy`
- require `--remote`
- support `--name` and `--provider`
- push spec over HTTP
- print deployment endpoints

Out of scope:

- `--always-on`
- config-file target resolution
- REPL

### Phase 2: Platform binary packaging

Scope:

- add optional platform packages
- wire `@fireline/cli` `optionalDependencies`
- add CI cross-compile + package publish matrix
- add install-time smoke tests for `npx fireline --help`

### Phase 3: Always-on wiring

Scope:

- add `--always-on` to `deploy`
- send `lifecycle.alwaysOn` to hosted Fireline
- hosted instance lowers that bit to `AlwaysOnDeploymentSubscriber`

Non-goal:

- no new lifecycle primitive in the CLI or hosted API

### Phase 4: `--repl` + hosted-instance auth

Scope:

- implement ACP-backed REPL
- add `--token`
- add `fireline.config.ts` target resolution
- add `--target <name>`
- wire env-token lookup

This phase is last because it sits on top of the already-working local and remote launch paths.

## 5. Validation Checklist

- [ ] `fireline run` behavior remains unchanged
- [ ] `fireline deploy <file> --remote <url>` can deploy a serialized Harness over HTTP
- [ ] `--always-on` is transmitted as deployment policy only
- [ ] always-on lowering references `AlwaysOnDeploymentSubscriber`, not a second primitive
- [ ] platform packages make `npx fireline` work without local `cargo build`
- [ ] `--repl` opens an ACP session instead of printing a stub message
- [ ] target resolution and token lookup are deterministic and documented
- [ ] `packages/fireline/README.md` and [docs/guide/cli.md](../guide/cli.md) are updated when phases land

## 6. Architect Review Checklist

- [ ] `deploy` stays thin and does not become a second configuration language
- [ ] `--always-on` is treated as policy input to hosted Fireline, not client-side supervision
- [ ] the hosted deploy API shape is narrow enough to evolve with the in-flight hosted deployment work
- [ ] platform packaging matches the already-shipped `resolve-binary.ts` lookup contract
- [ ] `--repl` remains a debug shell, not a large terminal subsystem
- [ ] config-file target resolution stays aligned with [deployment-and-remote-handoff.md](./deployment-and-remote-handoff.md)
- [ ] hosted deploy semantics stay consistent with [hosted-fireline-deployment.md](./hosted-fireline-deployment.md)

## References

- [packages/fireline/src/cli.ts](../../packages/fireline/src/cli.ts)
- [packages/fireline/src/resolve-binary.ts](../../packages/fireline/src/resolve-binary.ts)
- [packages/fireline/README.md](../../packages/fireline/README.md)
- [docs/guide/cli.md](../guide/cli.md)
- [declarative-agent-api-design.md](./declarative-agent-api-design.md)
- [deployment-and-remote-handoff.md](./deployment-and-remote-handoff.md)
- [hosted-fireline-deployment.md](./hosted-fireline-deployment.md)
- [durable-subscriber.md](./durable-subscriber.md)
- [durable-subscriber-execution.md](./durable-subscriber-execution.md)
