# Sandbox Providers

Provider selection is a typed discriminated union. `sandbox({ provider,
...providerSpecificFields })` gives you autocomplete and compile-time
validation of the fields that apply to each provider.

See:

- [packages/client/src/types.ts](../../packages/client/src/types.ts)
  — the `SandboxProviderConfig` type
- [crates/fireline-sandbox/src/provider_dispatcher.rs](../../crates/fireline-sandbox/src/provider_dispatcher.rs)
  — runtime dispatch

## The type

```ts
import type { SandboxProviderConfig } from '@fireline/client'

// SandboxProviderConfig =
//   | { provider?: 'local' }
//   | { provider: 'docker'; image?: string }
//   | { provider: 'microsandbox' }
//   | { provider: 'anthropic'; model?: string }
```

`sandbox({...})` accepts the base sandbox fields (`resources`,
`envVars`, `labels`) merged with the provider-specific variant. If you
pass `provider: 'docker'` you can also pass `image`; if you pass
`provider: 'anthropic'` you can pass `model`; the other variants have
no extra fields today.

## Local subprocess

Source:
[crates/fireline-sandbox/src/providers/local_subprocess.rs](../../crates/fireline-sandbox/src/providers/local_subprocess.rs)

```ts
sandbox({ provider: 'local' })
// or — `local` is the default:
sandbox()
```

Spawns another `fireline` process locally, waits for a structured
`FIRELINE_READY` line on stdout, mounts resources before launch, returns
ACP and state endpoints.

## Docker

Source:
[crates/fireline-sandbox/src/providers/docker.rs](../../crates/fireline-sandbox/src/providers/docker.rs)

```ts
sandbox({ provider: 'docker' })
sandbox({ provider: 'docker', image: 'node:22-slim' })
```

Builds or reuses a Fireline runtime image, starts a container, waits for
the `FIRELINE_READY` readiness line, supports mounted resources. The
optional `image` field hints the container image name to the host.

## Anthropic managed agents

Source:
[crates/fireline-sandbox/src/providers/anthropic.rs](../../crates/fireline-sandbox/src/providers/anthropic.rs)

```ts
sandbox({ provider: 'anthropic' })
sandbox({ provider: 'anthropic', model: 'claude-sonnet-4-6' })
```

Important facts:

- feature-gated behind `anthropic-provider` on the Rust side
- wired into the host control plane only when that feature is enabled
- requires `ANTHROPIC_API_KEY` in the host environment
- currently rejects Fireline resource mounts

The optional `model` field forwards to the managed-agents API;
otherwise the provider maps the first `agent_command` element to the
model name, defaulting to `claude-sonnet-4-6`.

## Microsandbox

Source:
[crates/fireline-sandbox/src/microsandbox.rs](../../crates/fireline-sandbox/src/microsandbox.rs)

```ts
sandbox({ provider: 'microsandbox' })
```

Important caveat:

- `MicrosandboxSandbox` exists behind the `microsandbox-provider`
  feature flag
- it is a lower-level sandbox primitive
- it is **not** currently wired into `ProviderDispatcher`

The variant exists in the TypeScript discriminated union so callers can
spell the provider name ahead of control-plane support landing. Until
dispatcher wiring is in place, the host rejects
`provider: 'microsandbox'` at runtime.

## Type-level checks

Because `SandboxProviderConfig` is a discriminated union, the following
mistakes fail at compile time:

```ts
// TS error — `image` only exists on the docker variant
sandbox({ provider: 'local', image: 'node:22' })

// TS error — unknown provider string
sandbox({ provider: 'kubernetes' })

// TS error — `model` only exists on the anthropic variant
sandbox({ provider: 'docker', model: 'claude-sonnet-4-6' })
```

## Known gap

The TS surface still does not reflect which providers are actually
configured on a given host. Calling `sandbox({ provider: 'anthropic' })`
against a host built without the `anthropic-provider` feature fails at
runtime, not at compile time.
