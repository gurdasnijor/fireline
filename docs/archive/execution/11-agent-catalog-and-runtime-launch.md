# 11. Agent Catalog And Runtime Launch

## Goal

Add a general-purpose Fireline path for:

- discovering ACP agents from the public registry
- resolving a launchable distribution for the chosen runtime
- creating a runtime from a catalog-selected agent

The browser harness is the first consumer, not the architectural reason for the
feature.

## What This Slice Proves

1. Fireline can normalize the ACP registry into a stable local catalog shape.
2. Fireline can resolve a catalog entry into a runnable local-provider command.
3. `client.host.create(...)` can launch by agent reference instead of only raw
   command.
4. A browser-facing control surface can drive that flow without hardcoding the
   terminal command.

## Current Scope

Implemented:

- ACP registry fetch + normalization in `@fireline/client`
- local command entries for private/dev agents
- resolution for `command`, `npx`, and `uvx`
- browser harness control API for:
  - `GET /api/agents`
  - `GET /api/runtime`
  - `POST /api/runtime`
  - `DELETE /api/runtime`
- browser harness runtime selection UI

Deferred:

- binary archive install/caching
- provider-specific remote installation
- agent provenance fields on `RuntimeDescriptor`
- Flamecast integration

## Why This Matters

This pays down integration risk in two ways:

- Fireline no longer assumes one hardcoded terminal agent in the dev harness.
- The launch path now uses the same discovery + resolution model we want for
  real control-plane consumers later.
