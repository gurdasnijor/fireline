# Environment and Config

Reference for the code-owned configuration surface around Fireline host URLs,
state-stream URLs, sandbox config, credential refs, and CLI env overrides.
This page stays on API mechanics. For the observation model, use
[Observation](../observation.md). For the state package itself, use
[`@fireline/state`](./state.md).

This page does not treat example-local variables such as `AGENT_COMMAND` or
`REPO_PATH` as product API.

## Environment Variable Catalog

This section is the quick inventory: which env vars belong to Fireline itself,
which ones are operator-side inputs to examples or deploy helpers, and where
the checked-in templates live.

### Fireline CLI binary overrides

The CLI binary resolver in
[`packages/fireline/src/resolve-binary.ts`](../../../packages/fireline/src/resolve-binary.ts)
honors these variables before it falls back to packaged or `target/{release,debug}`
binaries:

- `FIRELINE_BIN` — overrides the `fireline` binary path
- `FIRELINE_STREAMS_BIN` — overrides the `fireline-streams` binary path
- `FIRELINE_AGENTS_BIN` — overrides the `fireline-agents` binary path

Each must point at an existing file. If set to a missing path, the CLI throws
instead of silently falling back.

```bash
FIRELINE_BIN="$PWD/target/debug/fireline" \
FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams" \
fireline run agent.ts
```

### Fireline host and state defaults

- `FIRELINE_URL` — default host URL for `fireline repl`; many examples also use
  it as their default `serverUrl`
- `FIRELINE_STREAM_URL` — default state-stream URL read by
  `fireline.db(...)` when `stateStreamUrl` is omitted in Node

```bash
FIRELINE_URL=http://127.0.0.1:4440 fireline repl
FIRELINE_STREAM_URL=http://127.0.0.1:7474/streams/state/demo node monitor.mjs
```

### Operator-side secret env

- `ANTHROPIC_API_KEY` — commonly resolved through
  `secretsProxy({ ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' } })`
- `TELEGRAM_BOT_TOKEN` — commonly resolved through
  `telegram({ token: { ref: 'env:TELEGRAM_BOT_TOKEN' } })`

These are shell env vars owned by the operator or deploy target. In Fireline
code they usually appear as host-owned refs like `env:ANTHROPIC_API_KEY`,
not as inline secret values.

```ts
const mw = secretsProxy({
  ANTHROPIC_API_KEY: {
    ref: 'env:ANTHROPIC_API_KEY',
    allow: 'api.anthropic.com',
  },
})
```

### `deploy/telegram/bridge.env.example`

Checked-in template:
[deploy/telegram/bridge.env.example](../../../deploy/telegram/bridge.env.example)

This template is for the operator-run Telegram bridge process. It currently
defines:

- `TELEGRAM_BOT_TOKEN`
- `TELEGRAM_CHAT_ID`
- `TELEGRAM_ALLOWED_USER_IDS`
- `BRIDGE_PORT`
- `BRIDGE_CALLBACK_WEBHOOK_PATH`
- `FIRELINE_URL`
- `FIRELINE_STATE_STREAM_URL`

`bridge.env` is local operator state and must not be committed. Note that
`FIRELINE_STATE_STREAM_URL` in this file is the bridge's durable-streams base
URL for health and routing checks, not the `fireline.db(...)` fallback variable
`FIRELINE_STREAM_URL`.

```bash
cp deploy/telegram/bridge.env.example deploy/telegram/bridge.env
```

### `deploy/observability/betterstack.env.example`

Checked-in template:
[deploy/observability/betterstack.env.example](../../../deploy/observability/betterstack.env.example)

This template is for operator-side OTLP export config. Per
[deploy/observability/README.md](../../../deploy/observability/README.md),
the Fireline host consumes standard `OTEL_EXPORTER_OTLP_*` env vars. The
checked-in Betterstack template currently defines:

- `OTEL_EXPORTER_OTLP_ENDPOINT`
- `OTEL_EXPORTER_OTLP_HEADERS`
- `OTEL_SERVICE_NAME`
- `OTEL_RESOURCE_ATTRIBUTES` (commented optional example)

```bash
cp deploy/observability/betterstack.env.example deploy/observability/betterstack.env
set -a; source deploy/observability/betterstack.env; set +a
```

## Host Connection Config

### `new Sandbox(options)`

```ts
new Sandbox(options: SandboxClientOptions)

interface SandboxClientOptions {
  serverUrl: string
  token?: string
}
```

Creates a control-plane client for provisioning a harness against a Fireline
host.

```ts
import { Sandbox } from '@fireline/client'

const client = new Sandbox({
  serverUrl: 'http://127.0.0.1:4440',
  token: process.env.FIRELINE_TOKEN,
})
```

### `harness.start(options)`

```ts
interface StartOptions {
  serverUrl: string
  token?: string
  name?: string
  stateStream?: string
  startupTimeoutMs?: number
}

harness.start(options: StartOptions): Promise<FirelineAgent>
```

Starts a composed harness against a host URL. `name` and `stateStream` override
the values baked into the harness spec for this launch. `startupTimeoutMs`
exists on the type surface today but is reserved for future wiring.

```ts
const handle = await harness.start({
  serverUrl: 'http://127.0.0.1:4440',
  name: 'reviewer',
  stateStream: 'demo-reviewer',
})
```

### `HarnessSpec.stateStream`

```ts
interface HarnessSpec<Name extends string = string> {
  kind: 'harness'
  name: Name
  sandbox: SandboxDefinition
  middleware: MiddlewareChain
  agent: AgentConfig
  stateStream?: string
}
```

Pins a durable state stream at spec level. `StartOptions.stateStream` overrides
it when you need a different stream at launch time.

```ts
const spec = {
  kind: 'harness',
  name: 'reviewer',
  sandbox: sandbox(),
  middleware: middleware([]),
  agent: agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
  stateStream: 'demo-reviewer',
} satisfies HarnessSpec<'reviewer'>
```

## State DB URL Config

### `fireline.db(options?)`

```ts
function db(options?: FirelineDbOptions): Promise<FirelineDB>

interface FirelineDbOptions {
  stateStreamUrl?: string
  headers?: Record<string, string>
}
```

If `stateStreamUrl` is omitted, the client wrapper reads
`process.env.FIRELINE_STREAM_URL` in Node and otherwise falls back to
`http://localhost:7474/streams/state/default`.

```ts
import fireline from '@fireline/client'

const db = await fireline.db({
  stateStreamUrl: handle.state.url,
  headers: { Authorization: 'Bearer demo-token' },
})
```

### `createFirelineDB(config)`

```ts
function createFirelineDB(config: FirelineDBConfig): FirelineDB

interface FirelineDBConfig {
  stateStreamUrl: string
  headers?: Record<string, string>
  signal?: AbortSignal
}
```

Lower-level state package entry point. Unlike `fireline.db(...)`, this does not
read `FIRELINE_STREAM_URL` or preload automatically.

```ts
import { createFirelineDB } from '@fireline/state'

const db = createFirelineDB({
  stateStreamUrl: 'http://127.0.0.1:7474/streams/state/demo',
})

await db.preload()
```

## Sandbox Config

### `sandbox(config?)`

```ts
function sandbox(config?: Omit<SandboxDefinition, 'kind'>): SandboxDefinition

type SandboxDefinition = {
  kind: 'sandbox'
  resources?: readonly ResourceRef[]
  envVars?: Readonly<Record<string, string>>
  fsBackend?: 'local' | 'streamFs'
  labels?: Readonly<Record<string, string>>
} & SandboxProviderConfig
```

Creates the serialized sandbox definition carried by `compose(...)`.

```ts
import { sandbox } from '@fireline/client'

const cfg = sandbox({
  provider: 'docker',
  image: 'node:22-slim',
  envVars: { LOG_LEVEL: 'debug' },
  fsBackend: 'streamFs',
  labels: { demo: 'reviewer' },
})
```

### `SandboxProviderConfig`

```ts
type SandboxProviderConfig =
  | { provider?: 'local' }
  | { provider: 'docker'; image?: string }
  | { provider: 'microsandbox' }
  | { provider: 'anthropic'; model?: string }
```

Current provider-specific launch options understood by the TypeScript surface.

```ts
const local = sandbox({ provider: 'local' })
const docker = sandbox({ provider: 'docker', image: 'node:22-slim' })
const hosted = sandbox({ provider: 'anthropic', model: 'claude-sonnet-4-5' })
```

### `SandboxDefinition.envVars`

```ts
envVars?: Readonly<Record<string, string>>
```

Optional environment variables forwarded in the provisioning request as
`envVars`.

```ts
const cfg = sandbox({
  envVars: {
    NODE_ENV: 'production',
    FEATURE_FLAG_X: '1',
  },
})
```

### `SandboxDefinition.fsBackend`

```ts
fsBackend?: 'local' | 'streamFs'
```

Selects the filesystem backend used by ACP file helpers inside the sandbox.

```ts
const cfg = sandbox({ fsBackend: 'streamFs' })
```

### `SandboxDefinition.labels`

```ts
labels?: Readonly<Record<string, string>>
```

Adds metadata labels to the sandbox provisioning request.

```ts
const cfg = sandbox({
  labels: {
    demo: 'telegram',
    owner: 'pm-b',
  },
})
```

## Credential and Secret Refs

### `CredentialRef`

```ts
type CredentialRef =
  | { kind: 'env'; var: string }
  | { kind: 'secret'; key: string }
  | { kind: 'oauthToken'; provider: string; account?: string }
```

Structured host-resolved credential reference used by capability attachments.

```ts
const credential: CredentialRef = {
  kind: 'secret',
  key: 'gh-pat',
}
```

### Tool Credential Shorthand Strings

`attachTools(...)` currently parses and validates these string forms for
tool credentials:

- `env:NAME`
- `secret:KEY`
- `oauth:provider`
- `oauth:provider:account`

```ts
import { attachTools } from '@fireline/client'

attachTools([
  {
    name: 'github',
    transport: 'mcp:https://example.com/mcp',
    credential: 'secret:gh-pat',
  },
])
```

Invalid prefixes are rejected. The parser currently expects `env:`, `secret:`,
or `oauth:`.

### `SecretBinding`

```ts
interface SecretBinding {
  ref: string
  allow?: string | readonly string[]
}
```

Maps a logical secret name to a host-resolved credential ref and an optional
allow-list of outbound domains.

```ts
const binding: SecretBinding = {
  ref: 'env:ANTHROPIC_API_KEY',
  allow: 'api.anthropic.com',
}
```

### `DurableSubscriberSecretRef`

```ts
interface DurableSubscriberSecretRef {
  ref: string
}
```

Used by `telegram(...)` and `webhook(...)` when the host should resolve a token
or header secret at delivery time. The TypeScript layer forwards `ref`
verbatim; current demos use the same `env:...` and `secret:...` conventions as
tool credentials.

```ts
const token = { ref: 'env:TELEGRAM_BOT_TOKEN' }
```

## Middleware Config

### `secretsProxy(bindings)`

```ts
function secretsProxy(
  bindings: Readonly<Record<string, SecretBinding>>,
): SecretsProxyMiddleware
```

Declares runtime credential injection without exposing plaintext to the agent.

```ts
import { secretsProxy } from '@fireline/client'

const mw = secretsProxy({
  ANTHROPIC_API_KEY: {
    ref: 'env:ANTHROPIC_API_KEY',
    allow: 'api.anthropic.com',
  },
})
```

Current status note:

- `secretsProxy()` is a real shipped TypeScript surface and remains the right
  declarative secret-injection API.
- The stage-side operator docs currently carry one live caveat: the
  `pi-acp-to-openclaw` demo asset temporarily runs without `secretsProxy()`
  while `mono-4t4` tracks an `env:*` forwarding bug on that specific spawned
  process path.
- Treat that as an operational gap on one demo flow, not as removal of the API
  itself. The package surface and example surfaces that already use
  `secretsProxy()` are still the authoritative docs contract.

### `telegram(options)`

```ts
function telegram(options: TelegramOptions): TelegramMiddleware

interface TelegramOptions {
  name?: string
  target?: string
  token?: string | DurableSubscriberSecretRef
  chatId?: string
  allowedUserIds?: readonly string[]
  scope?: 'tool_calls'
  apiBaseUrl?: string
  approvalTimeoutMs?: number
  pollIntervalMs?: number
  pollTimeoutMs?: number
  parseMode?: 'html' | 'markdown_v2'
  events?: readonly DurableSubscriberEventSelector[]
  keyBy?: Exclude<DurableSubscriberKeyStrategy, 'session'>
  retry?: DurableSubscriberRetryPolicy
}
```

Current live lowering requires `token`. Defaults today are
`scope: 'tool_calls'`, `parseMode: 'html'`, `pollIntervalMs: 1000`, and
`pollTimeoutMs: 30000`.

```ts
import { telegram } from '@fireline/client'

const mw = telegram({
  token: { ref: 'env:TELEGRAM_BOT_TOKEN' },
  chatId: '123456',
  scope: 'tool_calls',
})
```

### `webhook(options)`

```ts
function webhook(options: WebhookOptions): WebhookMiddleware

interface WebhookOptions {
  name?: string
  target?: string
  url?: string
  events: readonly DurableSubscriberEventSelector[]
  keyBy?: Exclude<DurableSubscriberKeyStrategy, 'session'>
  headers?: Readonly<Record<string, DurableSubscriberSecretRef>>
  retry?: DurableSubscriberRetryPolicy
}
```

Current live lowering requires a concrete `url`; target-only routing is not
lowered yet. Header values are passed as host-resolved refs, not plaintext.

```ts
import { webhook } from '@fireline/client'

const mw = webhook({
  target: 'slack-approvals',
  url: 'https://hooks.slack.com/services/demo',
  events: ['permission_request'],
  headers: {
    Authorization: { ref: 'secret:slack-webhook-auth' },
  },
})
```

## CLI Env and Flags

### `fireline run`

```bash
fireline run <file.ts> \
  [--port <n>] \
  [--repl] \
  [--streams-port <n>] \
  [--state-stream <name>] \
  [--name <name>] \
  [--provider <provider>]
```

Boots `fireline` plus `fireline-streams`, provisions the spec locally, and can
drop into the REPL.

```bash
fireline run agent.ts --state-stream demo-reviewer --repl
```

### `fireline build`

```bash
fireline build <file.ts> \
  [--target <cloudflare|docker|docker-compose|fly|k8s>] \
  [--state-stream <name>] \
  [--name <name>] \
  [--provider <provider>]
```

Builds a hosted Fireline image and can scaffold a target config file.

```bash
fireline build agent.ts --target fly --name reviewer
```

Implementation note: the CLI currently embeds the serialized harness as the
generated Docker build arg `FIRELINE_EMBEDDED_SPEC`. Treat that as generated
CLI output rather than a hand-authored public config file.

### `fireline deploy`

```bash
fireline deploy <file.ts> \
  --to <fly|cloudflare-containers|docker-compose|k8s> \
  [--state-stream <name>] \
  [--name <name>] \
  [--provider <provider>] \
  [-- <native-flags...>]
```

Builds first, then hands off to the target-native CLI.

```bash
fireline deploy agent.ts --to fly -- --remote-only
```

### `fireline repl [session-id]`

```bash
fireline repl [session-id]
```

If `FIRELINE_URL` is unset, the REPL defaults to
`http://127.0.0.1:4440`. The ACP URL is derived by switching to `ws:`/`wss:`
and appending `/acp`.

```bash
FIRELINE_URL=http://127.0.0.1:4440 fireline repl
```

### Binary Override Env Vars

The CLI currently honors these environment variables during binary lookup:

- `FIRELINE_BIN`
- `FIRELINE_STREAMS_BIN`
- `FIRELINE_AGENTS_BIN`

Each must point at an existing binary path. If set to a missing path, the CLI
throws instead of silently falling back.

```bash
FIRELINE_BIN="$PWD/target/debug/fireline" \
FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams" \
fireline run agent.ts
```

### `FIRELINE_STREAM_URL`

`@fireline/client` reads this env var only for `fireline.db(...)` when
`stateStreamUrl` is omitted and `process.env` exists.

```bash
FIRELINE_STREAM_URL=http://127.0.0.1:7474/streams/state/demo node monitor.mjs
```

### `FIRELINE_URL`

The CLI REPL reads this env var as its default Fireline host URL.

```bash
FIRELINE_URL=http://127.0.0.1:5440 fireline repl
```
