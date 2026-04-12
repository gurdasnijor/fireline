# Declarative Agent API — Design

> Concrete interfaces, CLI spec, and implementation plan for every gap in
> [`gaps-declarative-agent-api.md`](../gaps-declarative-agent-api.md).
>
> Date: 2026-04-12

---

## 1. CLI — `npx fireline run`

### Commands

```
npx fireline run <file>              Local dev — boots conductor + agent in-process
npx fireline run <file> --resume <stream>   Resume a previous session
npx fireline run <file> --provider docker   Override sandbox provider
npx fireline deploy <file> [flags]   Deploy to remote Fireline instance
```

### `run` flags

| Flag | Default | Description |
|------|---------|-------------|
| `--port` | `4440` | ACP listener port |
| `--provider` | `local` | Override `sandbox.provider` from spec |
| `--resume <stream>` | — | Resume session from named durable stream |
| `--state-stream <name>` | auto-generated | Explicit stream name for this run |
| `--no-open` | `false` | Don't print the ACP endpoint |

### `deploy` flags

| Flag | Default | Description |
|------|---------|-------------|
| `--remote <url>` | — | Fireline instance URL (required) |
| `--provider` | from spec | Provider override |
| `--always-on` | `false` | Keep sandbox alive between sessions |
| `--peer <name>` | — | Discover and peer with named agents |
| `--token` | `$FIRELINE_TOKEN` | Auth token for remote instance |

### How it works

```
npx fireline run agent.ts
```

1. Resolve the platform-specific Rust binary (see §7)
2. Load `agent.ts` via `tsx` — import the default export
3. Assert it's a `HarnessSpec` (has `kind: 'harness'`)
4. Serialize the spec to JSON
5. Spawn the Rust binary with `--spec-json <json> --mode embedded`
6. The Rust binary:
   - Boots durable-streams in-process (SQLite backend)
   - Builds the topology from the spec's middleware components
   - Spawns the agent command, connects via stdio
   - Opens an ACP WebSocket listener on `--port`
7. CLI prints: `ACP: ws://localhost:4440/v1/acp/agent`

The Rust binary already does steps 6a–6d. The CLI is a thin JS shim
that loads the spec file and invokes the binary.

### `start()` API change

```typescript
// Before — serverUrl required
const handle = await spec.start({ serverUrl: 'http://localhost:4440' })

// After — no args for local, `remote` for remote
const handle = await spec.start()                                    // local, in-process
const handle = await spec.start({ remote: 'https://team.fireline.dev' })  // remote
```

The change in `sandbox.ts`:

```typescript
export interface StartOptions {
  /** Remote Fireline instance URL. Omit for local embedded mode. */
  readonly remote?: string
  /** @deprecated Use `remote` instead. */
  readonly serverUrl?: string
  readonly token?: string
  readonly name?: string
  readonly stateStream?: string
}
```

`start()` with no `remote` (and no `serverUrl`) spawns the Rust binary
as a child process and connects to it via stdio. This is the same path
the CLI uses.

---

## 2. `secretsProxy()` — TypeScript middleware

### Type (`types.ts`)

```typescript
/** Credential reference — matches Rust CredentialRef. */
export type CredentialRef =
  | { readonly kind: 'env'; readonly var: string }
  | { readonly kind: 'secret'; readonly key: string }
  | { readonly kind: 'oauth'; readonly provider: string; readonly account?: string }

/** A single secret proxy entry. */
export interface SecretProxyEntry {
  /** Credential to resolve. String shorthand expands to CredentialRef. */
  readonly ref: CredentialRef | string
  /** Domain allow-list. The credential is only injected for requests to these domains. */
  readonly allow?: string | readonly string[]
  /** Injection scope. Default: 'session'. */
  readonly scope?: 'session' | 'perCall' | 'once'
}

/** Middleware spec for credential isolation. */
export interface SecretsProxyMiddleware {
  readonly kind: 'secretsProxy'
  readonly entries: Readonly<Record<string, SecretProxyEntry>>
}
```

Add `SecretsProxyMiddleware` to the `Middleware` union:

```typescript
export type Middleware =
  | TraceMiddleware
  | ApproveMiddleware
  | BudgetMiddleware
  | ContextInjectionMiddleware
  | PeerMiddleware
  | SecretsProxyMiddleware    // ← new
  | AttachToolsMiddleware     // ← new (§4)
```

### Helper (`middleware.ts`)

```typescript
/**
 * Builds a secrets-proxy middleware spec for credential isolation.
 *
 * @example
 * ```ts
 * secretsProxy({
 *   GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' },
 *   OPENAI_KEY:   { ref: 'env:OPENAI_API_KEY' },
 * })
 * ```
 */
export function secretsProxy(
  entries: Record<string, SecretProxyEntry | string>,
): SecretsProxyMiddleware {
  const normalized: Record<string, SecretProxyEntry> = {}
  for (const [name, entry] of Object.entries(entries)) {
    normalized[name] = typeof entry === 'string'
      ? { ref: entry }
      : entry
  }
  return { kind: 'secretsProxy', entries: normalized }
}
```

The string shorthand `'env:OPENAI_API_KEY'` is ergonomic sugar. It
expands at the `middlewareToComponents` boundary (see below).

### `middlewareToComponents` mapping (`sandbox.ts`)

```typescript
case 'secretsProxy': {
  const rules = Object.entries(middleware.entries).map(([envName, entry]) => ({
    target: { kind: 'envVar', name: envName },
    credentialRef: parseCredentialRef(entry.ref),
    scope: entry.scope ?? 'session',
    ...(entry.allow ? { allow: Array.isArray(entry.allow) ? entry.allow : [entry.allow] } : {}),
  }))
  return [{
    name: 'secrets_injection',
    config: { rules },
  }]
}
```

Where `parseCredentialRef` expands the string shorthand:

```typescript
function parseCredentialRef(ref: CredentialRef | string): CredentialRef {
  if (typeof ref !== 'string') return ref
  if (ref.startsWith('env:'))    return { kind: 'env', var: ref.slice(4) }
  if (ref.startsWith('secret:')) return { kind: 'secret', key: ref.slice(7) }
  if (ref.startsWith('oauth:')) {
    const [, provider, account] = ref.split(':')
    return { kind: 'oauth', provider, ...(account ? { account } : {}) }
  }
  // Bare string → treat as env var name
  return { kind: 'env', var: ref }
}
```

### Rust registration (`host_topology.rs`)

Add to `build_host_topology_registry`, after the `attach_tool` block:

```rust
.register_component("secrets_injection", {
    let context = context.clone();
    move |config| {
        let config = config
            .ok_or_else(|| anyhow!("topology component 'secrets_injection' requires config"))?;
        let parsed: SecretsInjectionConfig =
            serde_json::from_value(config.clone()).context("parse secrets_injection config")?;
        let resolver: Arc<dyn CredentialResolver> =
            Arc::new(LocalCredentialResolver::default());
        let rules = parsed.rules.into_iter().map(|rule| {
            InjectionRule {
                target: match rule.target {
                    SecretsTargetConfig::EnvVar { name } => InjectionTarget::EnvVar(name),
                    SecretsTargetConfig::McpServerHeader { server, header } =>
                        InjectionTarget::McpServerHeader { server, header },
                    SecretsTargetConfig::ToolArg { tool, arg_path } =>
                        InjectionTarget::ToolArg { tool, arg_path },
                },
                credential_ref: rule.credential_ref,
                scope: match rule.scope.as_deref() {
                    Some("perCall") => InjectionScope::PerCall,
                    Some("once") => InjectionScope::Once,
                    _ => InjectionScope::Session,
                },
            }
        }).collect();
        Ok(sacp::DynConnectTo::new(
            SecretsInjectionComponent::new(resolver, rules)
        ))
    }
})
```

New config types (add in `host_topology.rs`):

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsInjectionConfig {
    pub rules: Vec<SecretsRuleConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsRuleConfig {
    pub target: SecretsTargetConfig,
    pub credential_ref: CredentialRef,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub allow: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SecretsTargetConfig {
    EnvVar { name: String },
    McpServerHeader { server: String, header: String },
    ToolArg { tool: String, arg_path: String },
}
```

---

## 3. Provider discriminated union types

Replace `provider?: string` in `SandboxDefinition` with a discriminated
union. Each provider carries only the config fields the Rust provider
struct actually reads.

### Types (`types.ts`)

```typescript
export type SandboxProvider =
  | { readonly kind: 'local' }
  | { readonly kind: 'docker'; readonly image?: string; readonly dockerfile?: string }
  | { readonly kind: 'microsandbox' }
  | { readonly kind: 'anthropic'; readonly model?: string }

export interface SandboxDefinition {
  readonly kind: 'sandbox'
  readonly resources?: readonly ResourceRef[]
  readonly envVars?: Readonly<Record<string, string>>
  readonly provider?: SandboxProvider
  readonly labels?: Readonly<Record<string, string>>
  readonly fsBackend?: FsBackendConfig  // ← §5
}
```

Remove `image?: string` from `SandboxDefinition` — it moves into the
`docker` variant.

### Wire mapping (`sandbox.ts`)

`buildProvisionRequest` extracts the provider string and merges
provider-specific fields:

```typescript
function resolveProvider(provider?: SandboxProvider): {
  providerName: string
  providerConfig: Record<string, unknown>
} {
  if (!provider) return { providerName: 'local', providerConfig: {} }
  switch (provider.kind) {
    case 'local':
      return { providerName: 'local', providerConfig: {} }
    case 'docker':
      return { providerName: 'docker', providerConfig: cloneDefined({
        image: provider.image,
        dockerfile: provider.dockerfile,
      })}
    case 'microsandbox':
      return { providerName: 'microsandbox', providerConfig: {} }
    case 'anthropic':
      return { providerName: 'anthropic', providerConfig: cloneDefined({
        model: provider.model,
      })}
  }
}
```

### Backwards compatibility

The old `provider?: string` callers break at the type level. This is
intentional — it's a pre-1.0 API and the string was never validated.
Migration: `{ provider: 'docker' }` → `{ provider: { kind: 'docker' } }`.

---

## 4. `attachTools()` middleware

### Types (`types.ts`)

```typescript
/** Shorthand tool attachment — expands to a full CapabilityRef. */
export interface ToolAttachment {
  /** Tool name exposed to the agent. */
  readonly name: string
  /** Human-readable description for the agent. */
  readonly description?: string
  /** JSON Schema for tool input. Omit for schema-less tools. */
  readonly inputSchema?: Record<string, unknown>
  /** Transport: MCP server URL, peer runtime, or in-process component. */
  readonly transport: string | TransportRef
  /** Optional credential for authenticated tools. */
  readonly credential?: string | CredentialRef
}

export type TransportRef =
  | { readonly kind: 'mcpUrl'; readonly url: string }
  | { readonly kind: 'peerRuntime'; readonly hostKey: string }
  | { readonly kind: 'smithery'; readonly catalog: string; readonly tool: string }
  | { readonly kind: 'inProcess'; readonly componentName: string }

export interface AttachToolsMiddleware {
  readonly kind: 'attachTools'
  readonly tools: readonly ToolAttachment[]
}
```

### Helper (`middleware.ts`)

```typescript
/**
 * Attaches external tools to the agent's capability surface.
 *
 * @example
 * ```ts
 * attachTools([
 *   { name: 'github', transport: 'mcp:https://github-mcp.example.com',
 *     credential: 'secret:gh-pat' },
 * ])
 * ```
 */
export function attachTools(tools: readonly ToolAttachment[]): AttachToolsMiddleware {
  return { kind: 'attachTools', tools: [...tools] }
}
```

### `middlewareToComponents` mapping

```typescript
case 'attachTools': {
  const capabilities = middleware.tools.map(tool => ({
    descriptor: {
      name: tool.name,
      description: tool.description ?? '',
      inputSchema: tool.inputSchema ?? { type: 'object' },
    },
    transportRef: parseTransportRef(tool.transport),
    ...(tool.credential ? { credentialRef: parseCredentialRef(tool.credential) } : {}),
  }))
  return [{ name: 'attach_tool', config: { capabilities } }]
}
```

Where `parseTransportRef`:

```typescript
function parseTransportRef(ref: string | TransportRef): TransportRef {
  if (typeof ref !== 'string') return ref
  if (ref.startsWith('mcp:')) return { kind: 'mcpUrl', url: ref.slice(4) }
  if (ref.startsWith('peer:')) return { kind: 'peerRuntime', hostKey: ref.slice(5) }
  return { kind: 'mcpUrl', url: ref }
}
```

No Rust changes needed — `attach_tool` is already registered in the
topology with `AttachToolConfig { capabilities: Vec<CapabilityRef> }`.

---

## 5. `fsBackend` config

### Types (`types.ts`)

```typescript
export type FsBackendConfig = 'local' | 'streamFs'
```

Added to `SandboxDefinition` (shown in §3 above).

### `middlewareToComponents` mapping (`sandbox.ts`)

`fsBackend` is not middleware — it's a sandbox-level config. Wire it in
`buildTopology`:

```typescript
function buildTopology(
  middleware: readonly Middleware[],
  name: string,
  fsBackend?: FsBackendConfig,
): TopologySpec {
  const components = middleware.flatMap(entry => middlewareToComponents(entry, name))
  if (fsBackend) {
    components.push({
      name: 'fs_backend',
      config: fsBackend === 'streamFs' ? { kind: 'streamFs' } : { kind: 'local' },
    })
  }
  return { components }
}
```

Update `buildProvisionRequest` to pass `config.sandbox.fsBackend`
through.

No Rust changes — `fs_backend` is already registered in the topology.

---

## 6. `peer()` config fix

The current `middlewareToComponents` drops the `peers` array. Fix:

```typescript
// Before (sandbox.ts line ~167)
case 'peer':
  return [{ name: 'peer_mcp' }]

// After
case 'peer':
  return [{
    name: 'peer_mcp',
    ...(middleware.peers?.length
      ? { config: { peers: [...middleware.peers] } }
      : {}),
  }]
```

That's it. The Rust `PeerComponent` already reads `peers` from config.

---

## 7. npm binary packaging

Ship the Rust binary as platform-specific npm optional dependencies,
the same pattern used by `esbuild`, `turbo`, `@biomejs/biome`, and
`@anthropic-ai/claude-code`.

### Package structure

```
packages/
├── fireline/                         # Main package — the CLI entry point
│   ├── package.json                  # bin: { fireline: "./bin/fireline" }
│   ├── bin/fireline                  # JS shim that resolves the platform binary
│   └── src/
│       ├── cli.ts                    # run/deploy command parsing (commander)
│       └── resolve-binary.ts         # Find platform binary
│
├── fireline-darwin-arm64/            # macOS ARM
│   ├── package.json                  # os: ["darwin"], cpu: ["arm64"]
│   └── bin/fireline                  # Native binary
│
├── fireline-darwin-x64/              # macOS Intel
├── fireline-linux-arm64/             # Linux ARM
├── fireline-linux-x64/              # Linux x86_64
└── fireline-win32-x64/               # Windows x86_64
```

### Main `package.json`

```json
{
  "name": "@fireline/cli",
  "bin": { "fireline": "./bin/fireline" },
  "optionalDependencies": {
    "@fireline/cli-darwin-arm64": "workspace:*",
    "@fireline/cli-darwin-x64": "workspace:*",
    "@fireline/cli-linux-arm64": "workspace:*",
    "@fireline/cli-linux-x64": "workspace:*",
    "@fireline/cli-win32-x64": "workspace:*"
  }
}
```

### Binary resolution (`resolve-binary.ts`)

```typescript
import { execFileSync } from 'node:child_process'
import { existsSync } from 'node:fs'
import { join } from 'node:path'

const PLATFORM_PACKAGES: Record<string, string> = {
  'darwin-arm64':  '@fireline/cli-darwin-arm64',
  'darwin-x64':    '@fireline/cli-darwin-x64',
  'linux-arm64':   '@fireline/cli-linux-arm64',
  'linux-x64':     '@fireline/cli-linux-x64',
  'win32-x64':     '@fireline/cli-win32-x64',
}

export function resolveFirelineBinary(): string {
  const key = `${process.platform}-${process.arch}`
  const pkg = PLATFORM_PACKAGES[key]
  if (!pkg) throw new Error(`Unsupported platform: ${key}`)

  try {
    const pkgDir = require.resolve(`${pkg}/package.json`)
    const binPath = join(pkgDir, '..', 'bin', 'fireline')
    if (existsSync(binPath)) return binPath
  } catch {}

  // Fallback: check if fireline is on PATH (dev mode)
  try {
    execFileSync('fireline', ['--version'], { stdio: 'ignore' })
    return 'fireline'
  } catch {}

  throw new Error(
    `Could not find fireline binary for ${key}. ` +
    `Install @fireline/cli or ensure the fireline binary is on PATH.`
  )
}
```

### CI build matrix

The release workflow cross-compiles for each target using
`cargo build --release --target <triple>`, then copies the binary into
the corresponding npm package directory. Standard GitHub Actions matrix:

```yaml
strategy:
  matrix:
    include:
      - target: aarch64-apple-darwin
        os: macos-latest
        npm-package: fireline-darwin-arm64
      - target: x86_64-apple-darwin
        os: macos-latest
        npm-package: fireline-darwin-x64
      - target: aarch64-unknown-linux-gnu
        os: ubuntu-latest
        npm-package: fireline-linux-arm64
      - target: x86_64-unknown-linux-gnu
        os: ubuntu-latest
        npm-package: fireline-linux-x64
      - target: x86_64-pc-windows-msvc
        os: windows-latest
        npm-package: fireline-win32-x64
```

---

## 8. File change summary

### TypeScript (`packages/client/`)

| File | Changes |
|------|---------|
| `src/types.ts` | Add `SecretsProxyMiddleware`, `AttachToolsMiddleware`, `CredentialRef`, `TransportRef`, `ToolAttachment`, `SecretProxyEntry`, `SandboxProvider` union, `FsBackendConfig`. Update `Middleware` union. Replace `provider?: string` with `provider?: SandboxProvider` in `SandboxDefinition`. Add `fsBackend?` to `SandboxDefinition`. Update `StartOptions` to make `serverUrl` optional + add `remote`. |
| `src/middleware.ts` | Add `secretsProxy()`, `attachTools()` helpers. |
| `src/sandbox.ts` | Add `'secretsProxy'` and `'attachTools'` cases to `middlewareToComponents`. Add `parseCredentialRef`, `parseTransportRef`, `resolveProvider` helpers. Fix `'peer'` case to pass peers config. Update `buildTopology` for `fsBackend`. Update `start()` to support no-args local mode. |
| `src/resources.ts` | No changes needed. |

### Rust (`crates/`)

| File | Changes |
|------|---------|
| `fireline-harness/src/host_topology.rs` | Register `"secrets_injection"` component. Add `SecretsInjectionConfig`, `SecretsRuleConfig`, `SecretsTargetConfig` config types. ~40 LOC. |
| `fireline-harness/src/secrets.rs` | No changes — implementation already complete. |
| `fireline-tools/src/lib.rs` | No changes — `CredentialRef`, `CapabilityRef` already exist. |
| `fireline-sandbox/src/providers/` | No changes — all 4 providers already exist. |

### New packages

| Package | Purpose |
|---------|---------|
| `packages/fireline/` | CLI entry point — `npx fireline run/deploy`. ~200 LOC. |
| `packages/fireline-{platform}/` | Platform-specific binary packages (5 targets). Build artifacts only. |

### Estimated LOC by gap

| Gap | TS | Rust | Total |
|-----|---:|-----:|------:|
| D1 CLI + `start()` | ~200 | 0 | ~200 |
| D2 `secretsProxy()` | ~45 | ~40 | ~85 |
| D3 Provider types | ~35 | 0 | ~35 |
| D4 `attachTools()` | ~30 | 0 | ~30 |
| D5 `fsBackend` | ~15 | 0 | ~15 |
| D6 `peer()` fix | ~5 | 0 | ~5 |
| D7 npm packaging | ~50 | 0 | ~50 |
| **Total** | **~380** | **~40** | **~420** |

### Implementation order

1. **D6 `peer()` fix** — 5 LOC, unblocks multi-agent. Ship immediately.
2. **D2 `secretsProxy()`** — Unblocks the single biggest README lie.
   TS types + helper + mapping + Rust registration.
3. **D3 Provider types** — Makes the "run anywhere" story
   self-documenting. Types only, no runtime change.
4. **D4 `attachTools()`** — Unblocks MCP tool injection. TS-only.
5. **D5 `fsBackend`** — Unblocks stream-FS. TS-only.
6. **D1 CLI + npm packaging** — The big one. Depends on D2-D6 being
   done so the CLI can demo the full compose story.

After D6 + D2, the north star scenario's middleware works end-to-end.
After D1, the entire `npx fireline run agent.ts` flow works.
