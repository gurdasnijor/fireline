# Sandbox Providers

Provider selection is currently a string on `sandbox({ provider: ... })`.

That is convenient but under-typed. The TS client does not yet have provider-specific config objects.

See:

- [packages/client/src/types.ts](../../packages/client/src/types.ts)
- [crates/fireline-sandbox/src/provider_dispatcher.rs](../../crates/fireline-sandbox/src/provider_dispatcher.rs)

## Local subprocess

Source:

- [crates/fireline-sandbox/src/providers/local_subprocess.rs](../../crates/fireline-sandbox/src/providers/local_subprocess.rs)

What it does:

- spawns another `fireline` process locally
- waits for a structured `FIRELINE_READY` line on stdout
- mounts resources before launch
- returns ACP and state endpoints for the child sandbox

Use it with:

```ts
sandbox({ provider: 'local' })
```

It is also the default when no provider is specified.

## Docker

Source:

- [crates/fireline-sandbox/src/providers/docker.rs](../../crates/fireline-sandbox/src/providers/docker.rs)

What it does:

- builds or reuses a Fireline runtime image
- starts a container
- waits for the same `FIRELINE_READY` readiness line
- supports mounted resources

Use it with:

```ts
sandbox({ provider: 'docker' })
```

## Anthropic managed agents

Source:

- [crates/fireline-sandbox/src/providers/anthropic.rs](../../crates/fireline-sandbox/src/providers/anthropic.rs)

Important facts:

- feature-gated behind `anthropic-provider`
- wired into the host control plane only when that feature is enabled
- requires `ANTHROPIC_API_KEY` in the host environment
- currently rejects Fireline resource mounts

Use it with:

```ts
sandbox({ provider: 'anthropic' })
```

The provider maps the first `agent_command` element to the model name, defaulting to `claude-sonnet-4-6`.

## Microsandbox

Source:

- [crates/fireline-sandbox/src/microsandbox.rs](../../crates/fireline-sandbox/src/microsandbox.rs)

Important caveat:

- `MicrosandboxSandbox` exists behind the `microsandbox-provider` feature flag
- it is a lower-level sandbox primitive
- it is **not** currently wired into `ProviderDispatcher`

So while the repo has Microsandbox code, the current control-plane path does not let you provision it with `sandbox({ provider: 'microsandbox' })`.

Treat Microsandbox as existing scaffolding, not a currently wired control-plane provider.

## Known provider gap

The stringly-typed `provider?: string` field means the TS client cannot statically express provider-specific requirements yet.

Today:

```ts
sandbox({ provider: 'docker' })
```

works because the control plane matches the string to a Rust provider at runtime.

What is still missing:

- typed provider discriminated unions on the TS side
- typed provider-specific options
- a TS surface that reflects which providers are actually configured on a given host
