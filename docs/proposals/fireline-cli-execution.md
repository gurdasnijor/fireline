# Fireline CLI Production-Readiness Gap Analysis + Design

> Status: gap-analysis + follow-on design
> Date: 2026-04-12
> Scope: `packages/fireline/` production-readiness work after the shipped `run` command, reshaped against the tiered deploy model in [`hosted-deploy-surface-decision.md`](./hosted-deploy-surface-decision.md) (`77e007d`)

This is a small gap document, not a full subsystem execution plan. The base CLI already exists in `packages/fireline/`. The remaining work is to close the production gap around OCI packaging, target-native deployment ergonomics, Tier C spec publishing, and the small operator conveniences that still sit behind stubs.

The hosted runtime model lives in [hosted-fireline-deployment.md](./hosted-fireline-deployment.md). This doc is only about the CLI surface needed to target that model cleanly.

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

- no `build` subcommand
- no `deploy` thin wrapper for target-native tooling
- no `push` command for Tier C spec-stream publishing
- no published platform binary packages
- `--repl` is still a stub that only prints a message
- no real automated CLI tests yet; `packages/fireline/package.json` still has `"test": "echo 'no tests yet'"`

## 2. Gap

### 2.1 `fireline build <file>`

The CLI can boot a local control plane, but it cannot yet produce the Tier A artifact: an OCI build context with the deployment spec embedded at build time.

### 2.2 `fireline deploy --to <platform>`

The tiered deploy decision removed any Fireline-owned deploy HTTP surface. What remains useful is a thin wrapper around target-native tooling such as `fly deploy`, `docker push`, `kubectl apply`, or equivalent platform commands.

### 2.3 `fireline push <file> --to <stream-url>`

Tier C needs a CLI verb, but not for Phase 1. Once `DeploymentSpecSubscriber` exists, the CLI should be able to append a spec resource to durable-streams. That is a stream write, not a deploy API.

### 2.4 Platform binary packaging

`resolve-binary.ts` already knows the desired packaging scheme, but the platform packages are still lookup stubs. Today `npx fireline` works only against a locally built Rust workspace.

### 2.5 `--repl`

The flag exists, but it does not connect to ACP or provide an interactive prompt loop. The current behavior is only "print the ACP URL and wait."

### 2.6 Warm-By-Default Hosted Behavior

Hosted deploys stay warm by default via the `AlwaysOnDeploymentSubscriber` substrate. Cold-start opt-out is not in scope for the initial ship.

For the CLI, that means the deploy surface passes hosted-deploy intent through to the tier that consumes it and does not grow a separate lifecycle switch for warmth behavior.

## 3. Proposed Design

### 3.1 `fireline build <file.ts>`

Add a codegen-first subcommand:

```bash
fireline build <file.ts> [--target <platform>] [--out <dir>]
```

Behavior:

1. Load the default-exported Harness exactly like `run`.
2. Serialize the compose spec into the hosted deployment manifest.
3. Write an OCI build context that embeds the spec into the hosted Fireline image layer.
4. If `--target` is provided, optionally scaffold target-specific config such as `fly.toml`, `Dockerfile`, or `k8s.yaml`.
5. Print artifact locations and stop.

Constraints:

- no network calls
- no registry push
- no platform deploy invocation
- no Fireline deploy HTTP

Phase 1 stops here on purpose. The build verb should stand on its own before any wrapper deploy UX exists.

### 3.2 `fireline deploy <file.ts> --to <platform>`

Add an optional thin wrapper:

```bash
fireline deploy <file.ts> --to <platform>
```

Behavior:

1. Run `fireline build`.
2. Resolve a platform adapter or plugin for `<platform>`.
3. Hand the generated OCI image reference and scaffolded config to target-native tooling.
4. Stream the native tool output back to the user.

This is explicitly not a Fireline protocol. It is a convenience wrapper over platform-native deployment commands.

Adapter shape to document, implementation deferred:

- `prepare(buildOutput): Promise<PreparedDeploy>`
- `exec(preparedDeploy): Promise<number>`
- optional config scaffolding hooks per platform

Examples:

- Fly.io: `fly deploy`
- Docker: `docker build` / `docker push` / `docker run`
- Kubernetes: `kubectl apply`

### 3.3 `fireline push <file.ts> --to <stream-url>`

Add a deferred Tier C verb:

```bash
fireline push <file.ts> --to <stream-url>
```

Behavior:

1. Load and serialize the compose spec.
2. Wrap it in the Tier C resource envelope expected by `DeploymentSpecSubscriber`.
3. Append it to the target durable stream.
4. Print the stream resource and append result.

Initial resolution should stay explicit:

- require `--to <stream-url>`
- use durable-streams auth, not a Fireline deploy token
- defer named targets in `fireline.config.ts` until Tier C is live

### 3.4 Warm-By-Default Treatment

Hosted deploys stay warm by default via the `AlwaysOnDeploymentSubscriber` substrate. Cold-start opt-out is not in scope for the initial ship.

That means:

- `fireline build` emits the hosted deployment artifact without inventing a separate warmth flag
- `fireline deploy --to <platform>` hands that artifact to the target platform unchanged
- `fireline push` appends the Tier C deployment input without adding a second CLI lifecycle contract

The CLI surface should describe this default behavior plainly rather than pretending it is a user-toggled runtime bit.

### 3.5 Platform binary packaging

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

`@fireline/cli` then lists them as `optionalDependencies`, matching the lookup contract already assumed by `resolve-binary.ts`.

### 3.6 `--repl`

`--repl` should become a real ACP shell, not a placeholder.

Proposed behavior:

- after `run`, connect to ACP
- read prompt lines from stdin
- send them as prompt requests
- stream assistant text to stdout
- `Ctrl+C` exits the REPL and then runs the normal teardown path

This should stay intentionally small. It is a debug/operator convenience, not a full TUI.

## 4. Phased Rollout

### Phase 1: `fireline build` codegen only

**Scope**

- Gate note: depends on [`hosted-deploy-surface-decision.md`](./hosted-deploy-surface-decision.md) (`77e007d`)
- Add `build`
- Load the compose spec exactly like `run`
- Generate the embedded-spec OCI build context
- Optionally scaffold target-specific config files
- Keep the command fully offline

**Out of scope**

- `deploy --to <platform>`
- `push`
- `--repl`

### Phase 2: `fireline deploy --to <platform>`

**Scope**

- Add a thin wrapper over `build`
- Define the platform adapter / plugin interface
- Start with one or two concrete adapters once the Tier A hosted MVP is green

**Gate**

- Tier A MVP from [hosted-fireline-deployment.md](./hosted-fireline-deployment.md) is green
- The wrapper delegates to a native platform tool rather than a Fireline deploy endpoint

### Phase 3: `fireline push`

**Scope**

- Add Tier C spec serialization
- Append to durable-streams with explicit `--to <stream-url>`
- Wire the command to `DeploymentSpecSubscriber`

**Gate**

- Tier A MVP from [hosted-fireline-deployment.md](./hosted-fireline-deployment.md) is green
- `DeploymentSpecSubscriber` exists and is replay-safe

### Phase 4: Packaging + REPL polish

**Scope**

- Publish platform binary packages
- Add install-time smoke coverage for `npx fireline --help`
- Turn `--repl` into a real ACP shell
- Add config polish only where it reduces repeated platform or stream flags

This phase stays last because it sits on top of already-working local, build, deploy-wrapper, and push flows.

## 5. Validation Checklist

- [ ] `fireline run` behavior remains unchanged
- [ ] `fireline build <file>` is codegen only and makes no network calls
- [ ] `fireline build` embeds the spec into the Tier A OCI image path
- [ ] `fireline deploy --to <platform>` is documented as a thin wrapper over target-native tooling
- [ ] `fireline push` is documented as a Tier C durable-streams append, not a deploy API
- [ ] No Fireline deploy HTTP endpoints remain in this proposal
- [ ] Warm-by-default hosted behavior is described as substrate-owned default behavior, not a CLI lifecycle flag
- [ ] platform packages make `npx fireline` work without a local Rust build
- [ ] `--repl` opens an ACP session instead of printing a stub message
- [ ] `packages/fireline/README.md` and [docs/guide/cli.md](../guide/cli.md) are updated when phases land

## 6. Architect Review Checklist

- [ ] Does `build` stay a codegen step rather than becoming a second configuration language?
- [ ] Does `deploy --to <platform>` stay a thin wrapper over target-native tooling?
- [ ] Does `push` remain an honest durable-streams append rather than a hidden control plane?
- [ ] Is warm-by-default hosted behavior clearly treated as substrate-owned default behavior rather than CLI lifecycle state?
- [ ] Does platform packaging still match the already-shipped `resolve-binary.ts` lookup contract?
- [ ] Does `--repl` remain a debug shell, not a large terminal subsystem?
- [ ] Do the staged CLI phases stay consistent with [hosted-fireline-deployment.md](./hosted-fireline-deployment.md)?

## References

- [packages/fireline/src/cli.ts](../../packages/fireline/src/cli.ts)
- [packages/fireline/src/resolve-binary.ts](../../packages/fireline/src/resolve-binary.ts)
- [packages/fireline/README.md](../../packages/fireline/README.md)
- [docs/guide/cli.md](../guide/cli.md)
- [hosted-deploy-surface-decision.md](./hosted-deploy-surface-decision.md)
- [hosted-fireline-deployment.md](./hosted-fireline-deployment.md)
- [durable-subscriber.md](./durable-subscriber.md)
- [durable-subscriber-execution.md](./durable-subscriber-execution.md)
