# QA: ACP Registry Live Validation

Date: 2026-04-12

Preflight build used an isolated target dir per contention rules:

```sh
CARGO_TARGET_DIR=/tmp/fireline-w13 cargo build --bin fireline --bin fireline-agents
```

Live registry used for all fetches:

```text
https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json
```

Important surface note:

- `node packages/fireline/bin/fireline.js agents <id>` is still not wired. It fails with `fireline: unexpected argument: <id>`.
- The shipped registry client lives in the Rust binary `fireline-agents add <id>`.
- Phase 3 compose fallback does work end to end; that is validated separately in scenario 6.

For isolation, all cache/install observations below used:

```sh
export HOME=/tmp/fireline-registry-home-2026-04-12
```

That produced:

- cache file: `/tmp/fireline-registry-home-2026-04-12/Library/Caches/fireline/agent-catalog.json`
- install root: `/tmp/fireline-registry-home-2026-04-12/Library/Application Support/fireline/agents`

## 1. Known agent ids

Verdict: Pass

Known ids chosen from the live registry:

- `claude-acp` (`npx` distribution)
- `amp-acp` (`binary` distribution)

Driver script:

```sh
HOME=/tmp/fireline-registry-home-2026-04-12 \
  /tmp/fireline-w13/debug/fireline-agents add claude-acp

HOME=/tmp/fireline-registry-home-2026-04-12 \
  /tmp/fireline-w13/debug/fireline-agents add amp-acp
```

Observed output:

```text
/tmp/fireline-registry-home-2026-04-12/Library/Application Support/fireline/agents/bin/claude-acp
/tmp/fireline-registry-home-2026-04-12/Library/Application Support/fireline/agents/bin/amp-acp
```

Evidence:

- Exit code for both commands: `0`
- Successful first fetch cached the entire live registry:

```text
version 1.0.0
agent_count 27
```

- Cached `claude-acp` descriptor:

```json
{
  "id": "claude-acp",
  "name": "Claude Agent",
  "version": "0.26.0",
  "distribution": {
    "binary": {},
    "npx": {
      "package": "@agentclientprotocol/claude-agent-acp@0.26.0",
      "args": [],
      "env": {}
    },
    "uvx": null
  }
}
```

- Cached `amp-acp` descriptor:

```json
{
  "id": "amp-acp",
  "name": "Amp",
  "version": "0.7.0",
  "distribution": {
    "binary": {
      "darwin-aarch64": {
        "archive": "https://github.com/tao12345666333/amp-acp/releases/download/v0.7.0/amp-acp-darwin-aarch64.tar.gz",
        "cmd": "./amp-acp"
      }
    },
    "npx": null,
    "uvx": null
  }
}
```

- Installed wrapper for `claude-acp`:

```bash
#!/usr/bin/env bash
set -euo pipefail
exec npx -y '@agentclientprotocol/claude-agent-acp@0.26.0' "$@"
```

- Installed wrapper for `amp-acp`:

```bash
#!/usr/bin/env bash
set -euo pipefail
exec "/tmp/fireline-registry-home-2026-04-12/Library/Application Support/fireline/agents/amp-acp/amp-acp" "$@"
```

- No live registry entries failed to parse. A successful fetch-and-cache deserialized the full catalog of `27` agents into `AgentCatalog`; if any entry had violated the `RemoteAgent` schema, the fetch would have failed before the cache file was written.

## 2. Unknown agent id

Verdict: Pass

Driver script:

```sh
HOME=/tmp/fireline-registry-home-2026-04-12 \
  /tmp/fireline-w13/debug/fireline-agents add does-not-exist-agent-id
```

Observed output:

```text
Error: ACP registry does not contain an agent with id 'does-not-exist-agent-id'
```

Evidence:

- Exit code: `1`
- No Rust panic or backtrace printed.
- Error is clean and specific to registry lookup.

## 3. Malformed agent id with spaces

Verdict: Pass

Driver script:

```sh
HOME=/tmp/fireline-registry-home-2026-04-12 \
  /tmp/fireline-w13/debug/fireline-agents add 'bad agent id'
```

Observed output:

```text
Error: ACP registry does not contain an agent with id 'bad agent id'
```

Evidence:

- Exit code: `1`
- Behavior is the same clean lookup failure as scenario 2.
- There is no separate format validator for agent ids; malformed ids are treated as unknown ids.

## 4. Network failure

Verdict: Fail

Gap:

- There is no `FIRELINE_REGISTRY_URL` or equivalent env override in the shipped CLI surface.
- `crates/fireline-tools/src/agent_catalog.rs` hardcodes `REGISTRY_URL` and only exposes `with_registry_url()` as an internal builder, not as a user-facing env/config hook.

Driver script used to simulate a network failure anyway:

```sh
HOME=/tmp/fireline-registry-home-netfail-2026-04-12 \
HTTPS_PROXY=http://127.0.0.1:9 \
HTTP_PROXY=http://127.0.0.1:9 \
  /tmp/fireline-w13/debug/fireline-agents add claude-acp
```

Observed output:

```text
Error: fetch ACP registry while resolving 'claude-acp'

Caused by:
    0: fetch ACP registry from https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json
    1: error sending request for url (https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json)
    2: client error (Connect)
    3: tunnel error: failed to create underlying connection
    4: tcp connect error
    5: Connection refused (os error 61)
```

Evidence:

- Exit code: `1`
- Error path is clean and contextual.
- Requested override mechanism is missing, so this scenario is still a product gap even though the fallback network error is readable.

## 5. Offline behavior and caching

Verdict: Fail

Driver script:

```sh
# first successful fetch populated cache
HOME=/tmp/fireline-registry-home-2026-04-12 \
  /tmp/fireline-w13/debug/fireline-agents add claude-acp

# second call with network deliberately broken
HOME=/tmp/fireline-registry-home-2026-04-12 \
HTTPS_PROXY=http://127.0.0.1:9 \
HTTP_PROXY=http://127.0.0.1:9 \
  /tmp/fireline-w13/debug/fireline-agents add claude-acp

# third call after waiting, still with network broken
sleep 2
HOME=/tmp/fireline-registry-home-2026-04-12 \
HTTPS_PROXY=http://127.0.0.1:9 \
HTTP_PROXY=http://127.0.0.1:9 \
  /tmp/fireline-w13/debug/fireline-agents add claude-acp
```

Observed output:

- Second call: success
- Third call after waiting: success

Evidence:

- Cache file mtime stayed unchanged across all three calls:

```text
mtime1=1776034577
mtime2=1776034577
mtime3=1776034577
```

- That proves the second call hit cache and did not refetch.
- There is no TTL mechanism in `AgentCatalogClient`; `load_cached_or_fetch()` only fetches when cache load fails, not when cache is stale.
- So the first half of the scenario passes, but the requested “refetch after TTL expiry” behavior does not exist today.

## 6. Downstream shape and Phase 3 compose fallback

Verdict: Pass

Driver script:

```sh
cat > packages/fireline/test-fixtures/registry-fallback-spec.ts <<'EOF'
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'

export default compose(
  sandbox({ labels: { test: 'registry-fallback' } }),
  middleware([trace()]),
  agent(['claude-acp']),
)
EOF

HOME=/tmp/fireline-registry-home-2026-04-12-phase3 \
FIRELINE_BIN=/tmp/fireline-w13/debug/fireline \
FIRELINE_STREAMS_BIN=/tmp/fireline-w13/debug/fireline-streams \
  node packages/fireline/bin/fireline.js run \
    packages/fireline/test-fixtures/registry-fallback-spec.ts \
    --port 17440 \
    --streams-port 19474
```

Observed output:

```text
durable-streams ready at http://127.0.0.1:19474/v1/stream

  ✓ fireline ready

    sandbox:   runtime:cee1f5d0-e3b3-474a-8167-4cfdfd00a331
    ACP:       ws://127.0.0.1:55004/acp
    state:     http://127.0.0.1:19474/v1/stream/fireline-state-runtime-cee1f5d0-e3b3-474a-8167-4cfdfd00a331
```

Evidence:

- The single-token `agent(['claude-acp'])` spec booted successfully and reached `fireline ready`.
- The run created the expected registry cache file and `claude-acp` wrapper under the isolated `HOME`.
- That is the Phase 3 fallback path working end to end: catalog lookup, install resolution, and runtime boot through `fireline run`.

## Summary

- Passed: 1, 2, 3, 6
- Failed: 4, 5

Main findings:

- Registry fetch, parse, cache, and install work against the live ACP registry for both `npx` and binary distributions.
- Unknown and malformed ids fail cleanly.
- Phase 3 compose fallback is live: `agent(['claude-acp'])` booted successfully through `fireline run`.
- The user-facing `fireline agents` wrapper is still missing; only `fireline-agents add <id>` works today.
- There is no registry URL override and no cache TTL/refetch behavior. Those are the main remaining operator gaps.
