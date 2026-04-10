# Agent Catalog And Launch

Fireline should treat agent discovery and runtime launch as separate concerns.

- the ACP registry says what can be launched
- Fireline resolves what should be launched in a chosen runtime
- the runtime descriptor records what was actually launched
- durable state records what happened after launch

## Sources

The first catalog source is the ACP agent registry:

- RFD: <https://agentclientprotocol.com/rfds/acp-agent-registry>
- aggregate: <https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json>

Fireline may also merge:

- local/private registry entries
- development-only local command entries

The browser harness uses that last category for `fireline-testy-load`.

## Layers

### 1. Catalog

Discovery only.

Fireline normalizes registry entries into a stable internal shape:

```ts
type AgentCatalogEntry = {
  source: 'registry' | 'local'
  id: string
  name: string
  version: string
  description?: string
  distributions: AgentDistribution[]
}
```

### 2. Resolver

The resolver chooses a launchable distribution for a specific runtime
environment.

Current local-provider support:

- `command`
- `npx`
- `uvx`

Current local-provider non-support:

- `binary` archive install is recognized but not implemented yet

That means binary-only agents appear in the catalog, but are marked
non-launchable until Fireline grows local binary install/caching.

### 3. Runtime launch

`RuntimeHost.create(...)` should accept either:

- a manual command
- a catalog agent reference

Current TypeScript shape:

```ts
await client.host.create({
  provider: 'local',
  agent: {
    source: 'catalog',
    agentId: 'codex-acp',
  },
})
```

The host resolves that to a runnable command before spawning Fireline.

### 4. Runtime truth

Once launched, the source of truth is no longer the registry entry.

The runtime descriptor is the launch record:

- runtime identity
- provider
- ACP URL
- state stream URL
- launched agent provenance

The durable state stream then becomes the execution truth.

## Why This Is Not Harness-Specific

The browser harness is only the first consumer.

The reusable primitives are:

- `@fireline/client` catalog client
- `@fireline/client` resolver
- `@fireline/client` host create with catalog agent refs

The harness adds only a thin local control API so a browser can invoke those
Node-side capabilities during development.

Flamecast or another control plane can consume the same catalog + resolver path
without reusing any harness code.

## First Consumer

The browser harness now:

- lists launchable catalog agents from `/api/agents`
- launches one selected agent into a local Fireline runtime via `/api/runtime`
- reconnects the browser to the stable `/acp` and `/v1/stream/:name` endpoints

That gives Fireline a real end-to-end integration harness without hardcoding the
terminal agent in the UI.
