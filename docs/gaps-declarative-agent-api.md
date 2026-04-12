# Declarative Agent API Gaps

> Fireline as the endpoint: a functional, declarative API for defining durable "black box" agents. Define a spec, run it anywhere, don't think about internals.
>
> Consumer: agent operators, deployers, anyone who wants to go from "local experiment" to "always-on cloud agent fleet" without writing glue code.
>
> Companion doc: [`gaps-platform-sdk.md`](gaps-platform-sdk.md) — imperative API gaps for building applications on top of Fireline.
>
> Date: 2026-04-12

---

## North star scenario

Every gap in this doc is measured against one question: **can we do this today?**

```typescript
// agent.ts — 15 lines, the entire agent definition
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve, budget, secretsProxy, peer } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
      GITHUB_TOKEN:      { ref: 'secret:gh-pat', allow: 'api.github.com' },
    }),
    peer(),
  ]),
  agent(['pi-acp']),
)
```

```bash
# Local dev — iterate on the agent definition
npx fireline run agent.ts
# → ACP: ws://localhost:4440/v1/acp/agent
# → Connect with any ACP client (pi-acp UI, use-acp, claude-code, etc.)

# Deploy to cloud — same file, provider override
npx fireline deploy agent.ts --provider anthropic --always-on
# → Session migrates. Secrets resolve from vault. Agent is always-on.
# → Durable waits: close laptop, approve from phone 5 hours later.

# Peer with other agents — OpenClaw-style agent fleet
npx fireline deploy reviewer.ts --provider anthropic --peer agent
# → Agents discover each other via the durable discovery stream.
# → Cross-agent prompting, shared lineage graph, unified observation.
```

**The flow from "local experiment" to "always-on cloud agent fleet with peer discovery" is the same 15-line file with a flag change.**

Concrete inspiration: take a local [pi-acp](https://github.com/svkozak/pi-acp) agent, instrument it with Fireline middleware, test locally, deploy to the cloud as an always-on [OpenClaw](https://github.com/openclaw/openclaw)-style agent that peers with others — sharing a message board, a lineage graph, and a unified observation surface.

---

## D1. The CLI is the primary interface — `npx fireline agent.ts`

The compose spec is pure data. The CLI is the runtime. There is no separate "server" to start.

**Current path to run a Fireline agent:**
1. `cargo build` the Rust workspace
2. Start a durable-streams server (`fireline-streams`)
3. Start a fireline host server (`fireline --port 4440 ...`)
4. Write a script that calls `.start({ serverUrl: 'http://localhost:4440' })`
5. Manually connect an ACP client
6. Start prompting

That's 6 steps, and step 4 forces the user to know about HTTP servers. If the conductor connects to the agent via stdio, there IS no server.

**What should exist:**

```bash
npx fireline agent.ts
```

The CLI:
1. Reads the default export (a compose spec — pure data)
2. Boots the conductor in-process
3. Spawns the agent, connects via stdio
4. Embeds durable streams in-process
5. Prints the ACP endpoint for any [ACP client](https://agentclientprotocol.com/get-started/clients) to connect

No server to start. No URL to know. The spec file IS the infrastructure.

```bash
# Local dev
npx fireline agent.ts
# → ACP: ws://localhost:4440/v1/acp/agent

# Resume a previous session
npx fireline agent.ts --state-stream my-session

# Override provider
npx fireline agent.ts --provider docker

# Deploy to cloud
npx fireline deploy agent.ts --provider anthropic --always-on
```

**This changes `start()` too.** The current `start({ serverUrl })` presupposes client-server. The correct API:

```typescript
// Default: local, in-process, no URL, no server
const agent = await spec.start()

// Remote: connect to an existing Fireline instance
const agent = await spec.start({ remote: 'https://team.fireline.dev' })
```

`start()` with no arguments boots everything locally. `remote` is the escape hatch for when you're targeting someone else's Fireline instance. `serverUrl` goes away.

**To fix:**
- Ship the fireline Rust binary as a platform-specific npm optional dependency (like esbuild, turbo, etc.)
- `npx fireline <file>` loads the TS file, extracts the default export, boots the runtime
- Conductor connects to agent via stdio — no HTTP server in the default path
- Durable streams embedded in-process
- `start()` with no arguments works locally; `start({ remote })` for remote instances

**Work:** ~200 LOC (CLI shim) + npm binary packaging + `start()` API revision. The binary already exists (`src/main.rs`).

---

## D2. `secretsProxy()` middleware does not exist

**README promises** (lines 94, 108, 115, 248):
```typescript
import { trace, approve, secretsProxy } from '@fireline/client/middleware'

secretsProxy({
  GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' },
  OPENAI_KEY:   { ref: 'secret:openai', allow: 'api.openai.com' },
})
```

**Client exports:** Nothing. `middleware.ts` has `trace`, `approve`, `budget`, `contextInjection`, `inject`, `peer`. No `secretsProxy`.

**Rust backend:** Fully implemented. `SecretsInjectionComponent` (754 lines in `crates/fireline-harness/src/secrets.rs`) with `CredentialResolver` trait, `InjectionRule`, `InjectionTarget::{EnvVar, McpServerHeader, ToolArg}`, session-scoped caching, `ConnectTo<Conductor>` wiring, `LocalCredentialResolver` (reads `~/.config/fireline/secrets.toml` + env fallback). However, the secrets component is connected manually in bootstrap, not via the topology registry. The TS middleware spec would need a new registered topology component on the Rust side too.

**Impact:** Cannot demo the "local → cloud handoff" story. Cannot demo credential isolation. Cannot demo the entire secrets-isolation section of the README. This is the single biggest gap between what the README shows and what the code delivers.

**To fix:**
- TS: Add `secretsProxy()` to `middleware.ts` (~20 LOC)
- TS: Add `middlewareToComponents` case in `sandbox.ts` (~20 LOC)
- Rust: Register `"secrets_injection"` in `host_topology.rs` (~30 LOC)

---

## D3. `sandbox({ provider })` has no type safety or provider-specific config

**Current:** `provider?: string` — untyped, no autocomplete, no provider-specific options.

**What should exist:**
```typescript
sandbox({ provider: 'local' })
sandbox({ provider: 'docker', image: 'node:22-slim' })
sandbox({ provider: 'microsandbox' })
sandbox({ provider: 'anthropic', model: 'claude-sonnet-4-20250514' })
```

**What Rust supports:** Four providers exist in `crates/fireline-sandbox/src/providers/` — `local_subprocess.rs`, `docker.rs`, `anthropic.rs`, plus microsandbox. Each has provider-specific config.

**To fix:** Make `provider` a discriminated union with provider-specific config fields. ~30 LOC (types only).

---

## D4. `attach_tool` / capability profiles have no TS middleware surface

**Rust supports:** `AttachToolComponent` with `CapabilityRef { descriptor, transport_ref, credential_ref }` triples. Registered as `"attach_tool"` in topology. This is Anthropic's Slice 17 capability profiles.

**TS exports:** Nothing. No `attachTools()` helper, no `CapabilityRef` type.

**What it would enable:**
```typescript
middleware([
  trace(),
  attachTools([
    { name: 'github', transport: 'mcp:github-mcp-server', credential: 'secret:gh-pat' },
    { name: 'linear', transport: 'mcp:linear-server', credential: 'secret:linear-key' },
  ]),
])
```

**To fix:** Add `attachTools()` to `middleware.ts`, add `CapabilityRef` types. ~25 LOC.

---

## D5. `fs_backend` has no TS middleware surface

**Rust supports:** `FsBackendComponent` with `FsBackendConfig::Local | FsBackendConfig::StreamFs`. Registered as `"fs_backend"` in topology.

**TS exports:** Nothing. The user cannot switch between local mounted files and stream-backed files from the client.

**What it would enable:**
```typescript
sandbox({
  resources: [localPath('.', '/workspace')],
  fsBackend: 'streamFs',  // files accessible from any host via durable stream
})
```

**To fix:** Add `fsBackend` to `SandboxDefinition`, wire it into `buildTopology()`. ~15 LOC.

---

## D6. `peer()` middleware doesn't wire peer names into topology

**Current:** `peer({ peers: ['agent:reviewer'] })` accepts peer names but `middlewareToComponents` ignores them — it just emits `{ name: 'peer_mcp' }` with no config.

**Rust side:** `PeerComponent` reads peer list from config and wires cross-agent MCP routing.

**To fix:** Pass `peers` through to the topology component config. ~5 LOC.

---

## Summary

| Gap | TS work | Rust work | Blocks |
|-----|---------|-----------|--------|
| **D1 `npx fireline run`** | ~200 LOC (CLI) | npm packaging | First-run experience |
| **D2 `secretsProxy()`** | ~40 LOC | ~30 LOC | Secrets, handoff, README truth |
| D3 Provider type safety | ~30 LOC (types) | None | "Run anywhere" story |
| D4 `attachTools()` | ~25 LOC | None | MCP tool injection |
| D5 `fsBackend` config | ~15 LOC | None | Stream-FS story |
| D6 `peer()` config wiring | ~5 LOC | None | Cross-agent routing |

## Recommended order

1. **D1 `npx fireline run`** — Without this, nobody gets past step 1.
2. **D2 `secretsProxy()`** — Without this, the README is lying and the handoff story is dead.
3. **D3 Provider types** — Makes the "run anywhere" story self-documenting.
4. **D6 `peer()` wiring** — 5 LOC fix that unblocks the multi-agent fleet story.
5. **D4 + D5** — Unblock MCP tool injection and stream-FS stories.

After D1 + D2 + D6, the north star scenario works end-to-end.
