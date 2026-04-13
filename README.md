# Fireline

[![License: Apache 2.0](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](https://www.apache.org/licenses/LICENSE-2.0)
[![CI](https://img.shields.io/github/actions/workflow/status/gurdasnijor/fireline/fireline-cli.yml?branch=main&label=ci)](https://github.com/gurdasnijor/fireline/actions/workflows/fireline-cli.yml)
[![Docs](https://img.shields.io/badge/docs-guide-0A66C2)](docs/guide/README.md)
[![GitHub stars](https://img.shields.io/github/stars/gurdasnijor/fireline?style=social)](https://github.com/gurdasnijor/fireline/stargazers)

**Durable infrastructure for AI agents that need to be visible, controllable, and crash-proof.**

Fireline gives your agent product three things teams usually build badly by hand: live visibility, hard control points, and durable execution.

[Guides](docs/guide/README.md) · [Examples](examples/) · [North-star demo](docs/demos/pi-acp-to-openclaw.md) · [Architecture](docs/architecture.md)

## The Problem

The moment you give an agent real tools, files, API keys, or production data, you hit three product problems:
**How do I know what the agent is doing? How do I stop it from doing something bad? What happens when it crashes mid-task?**

Logs arrive too late, prompt guardrails are suggestions, and most agent sessions die with the process that was running them. Fireline exists for the point where "call the model and hope" stops being enough.

## The Answer

- **Visibility.** Turn prompts, tool calls, approvals, and peer handoffs into live product data your UI can subscribe to. Start with the [Observation guide](docs/guide/observation.md).
- **Control.** Enforce approvals, budgets, secrets isolation, and peer routing as infrastructure middleware the agent cannot bypass. Start with [Middleware](docs/guide/middleware.md) and [Approvals](docs/guide/approvals.md).
- **Durability.** Keep sessions and long waits on the stream so work survives restarts, host moves, and human pauses. Start with [Durable subscribers](docs/guide/durable-subscriber.md), [Awakeables](docs/guide/awakeables.md), and [Multi-agent](docs/guide/multi-agent.md).

## See It In Action

**This is the working demo spec on current `main`: one ACP agent, wrapped with tracing, approvals, budgets, and peer discovery.**

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { approve, budget, peer, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    peer({ peers: ['reviewer'] }),
  ]),
  agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp']),
)
```

This is the same shape used in [docs/demos/assets/agent.ts](docs/demos/assets/agent.ts).

**Actual output today.**

```text
$ npx fireline run docs/demos/assets/agent.ts --port 8989
fireline: reusing fireline-streams at :7474

  ✓ fireline ready

    sandbox:   runtime:f2a2e08a-5cb3-4ecd-abc8-6ff0735a3687
    ACP:       ws://127.0.0.1:58996/acp
    state:     http://127.0.0.1:7474/v1/stream/fireline-state-runtime-f2a2e08a-5cb3-4ecd-abc8-6ff0735a3687

  Press Ctrl+C to shut down.

  To interact: npx fireline docs/demos/assets/agent.ts --repl
```

This capture is from [docs/demos/recordings/jessica-dryrun-2026-04-13.md](docs/demos/recordings/jessica-dryrun-2026-04-13.md). For the hosted-image path after local boot, use the [Local To Cloud guide](docs/guide/guides/local-to-cloud.md).

## What You Stop Hand-Building

<picture>
  <img alt="Before and after Fireline" src="assets/before-after.svg" width="100%">
</picture>

Without Fireline, a real agent usually means hand-building and maintaining:

- durable state storage with replay semantics: roughly 300 LOC
- an approval gate with rebuild-from-log behavior: roughly 150 LOC
- OpenTelemetry span emission with canonical ACP ids: roughly 200 LOC
- peer routing and lineage plumbing across agents: roughly 150 LOC

That is roughly **800 LOC of bespoke agent infrastructure** before you build any product UI.

With Fireline, it becomes:

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { approve, budget, peer, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    peer({ peers: ['reviewer'] }),
  ]),
  agent(['npx', '-y', '@agentclientprotocol/claude-agent-acp']),
)
```

That's it.

## Why It's Different

| Compared to | Good at | Where Fireline fits |
|---|---|---|
| **LangChain / CrewAI** | Authoring agent logic, chains, and tool use | Fireline is the runtime layer around the agent: observation, guardrails, durable state, and operator surfaces. |
| **Anthropic Managed Agents** | Zero-ops Claude-native hosting | Fireline is the fit when you want self-hosting, provider portability, model flexibility, or your own middleware and UI surface. |
| **Claude API directly** | Fastest path to a model call | Fireline is for the moment your agent needs tools, approvals, dashboards, or resume-after-crash behavior. |

## Features

<table>
  <tr>
    <td valign="top" width="50%">
      <img alt="Middleware pipeline" src="assets/middleware-pipeline.svg" width="100%">
      <br>
      <strong>Middleware</strong><br>
      Compose <code>trace()</code>, <code>approve()</code>, <code>budget()</code>, <code>peer()</code>, and <code>telegram()</code> as infrastructure rules the agent cannot route around.
      <br>
      <a href="docs/guide/middleware.md">Middleware guide</a>
    </td>
    <td valign="top" width="50%">
      <img alt="Secrets isolation" src="assets/secrets-isolation.svg" width="100%">
      <br>
      <strong>Secrets</strong><br>
      The design goal is credential injection without handing plaintext tokens to the agent; the live demo spec currently omits <code>secretsProxy()</code> while that path is finished post-demo.
      <br>
      <a href="docs/guide/middleware.md">Middleware guide</a> · <a href="docs/proposals/secrets-injection-component.md">Design detail</a>
    </td>
  </tr>
  <tr>
    <td valign="top" width="50%">
      <img alt="Durable approval flow" src="assets/durable-approval-flow.svg" width="100%">
      <br>
      <strong>Durable approvals</strong><br>
      Pause risky tool calls, render the decision in a dashboard or bot, and resume the same session after the approval lands.
      <br>
      <a href="docs/guide/approvals.md">Approvals guide</a> · <a href="docs/demos/fqa-approval-demo-capture.md">Replay capture</a>
    </td>
    <td valign="top" width="50%">
      <img alt="Multi-agent topology" src="assets/multi-agent-topology.svg" width="100%">
      <br>
      <strong>Multi-agent</strong><br>
      Put planner, reviewer, and helper agents on one durable discovery surface and keep lineage across peer hops instead of stitching logs together later.
      <br>
      <a href="docs/guide/multi-agent.md">Multi-agent guide</a> · <a href="docs/demos/peer-to-peer-demo-capture.md">Peer replay</a>
    </td>
  </tr>
  <tr>
    <td valign="top" colspan="2">
      <img alt="Durable wait" src="assets/durable-wait.svg" width="100%">
      <br>
      <strong>Durable waits</strong><br>
      Keep long human pauses and external completions on the stream instead of in a process-local callback. The shipped public surface now includes both approval gates and <a href="docs/guide/awakeables.md">awakeables</a> over the same canonical completion-key substrate.
      <br>
      <a href="docs/guide/durable-subscriber.md">Durable subscriber guide</a> · <a href="docs/guide/awakeables.md">Awakeables guide</a>
    </td>
  </tr>
</table>

## Quick Start

Today the honest quick start is from a repo checkout. The public `@fireline/cli` and `@fireline/client` npm packages are not published yet, so start with the repo-local `npx fireline` workflow:

```bash
git clone https://github.com/gurdasnijor/fireline.git
cd fireline
pnpm install
pnpm --filter @fireline/cli build
cargo build --bin fireline --bin fireline-streams

export FIRELINE_BIN="$PWD/target/debug/fireline"
export FIRELINE_STREAMS_BIN="$PWD/target/debug/fireline-streams"
```

```typescript
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([trace(), approve({ scope: 'tool_calls' })]),
  agent(['npx', '-y', '@anthropic-ai/claude-code-acp']),
)
```

```bash
npx fireline run agent.ts
```

That repo-local path is documented end to end in the [5-minute Quickstart](docs/guide/guides/quickstart.md). For the full CLI surface, use the [CLI guide](docs/guide/cli.md) and [Compose and Start](docs/guide/compose-and-start.md).

## Examples

| Example | Pattern | What it shows |
|---|---|---|
| [`examples/code-review-agent/`](examples/code-review-agent/) | Scoped code access | `compose` + `approve` + `secretsProxy` + reactive observation of pending approvals |
| [`examples/approval-workflow/`](examples/approval-workflow/) | Durable approvals | `FirelineAgent.resolvePermission()` + stream-backed approval checkpoints |
| [`examples/background-task/`](examples/background-task/) | Fire-and-forget | Provision, prompt, observe completion via stream subscription |
| [`examples/live-monitoring/`](examples/live-monitoring/) | Reactive dashboard | `useLiveQuery` across `sessions`, `promptTurns`, `permissions`, `chunks` |
| [`examples/multi-agent-team/`](examples/multi-agent-team/) | Multi-agent | `pipe(...)` plus shared-state observation across a team |
| [`examples/crash-proof-agent/`](examples/crash-proof-agent/) | Session resume | `stateStream` + `session/load` after sandbox death |
| [`examples/cross-host-discovery/`](examples/cross-host-discovery/) | Discovery | Two control planes, shared discovery stream, peer MCP routing |
| [`examples/flamecast-client/`](examples/flamecast-client/) | Platform client | Dashboard UI over `fireline.db` |

## Architecture Summary

Fireline splits agent systems into three planes:

- **Control** — define a serializable harness with `compose(...)`, then provision it
- **Session** — talk to the running agent over ACP
- **Observation** — treat the durable stream as the source of truth and materialize it into live state

The deeper crate map, proposal links, and verification notes live in [docs/architecture.md](docs/architecture.md). The current user-facing surface is indexed in [docs/guide/README.md](docs/guide/README.md).

## Built On

- [**durable-streams**](https://durablestreams.com) — append-only, replayable event streams
- [**ACP**](https://agentclientprotocol.com) — agent ↔ host communication
- [**sacp-conductor**](https://github.com/agentclientprotocol/rust-sdk) — Rust ACP conductor and middleware pipeline
- [**microsandbox**](https://github.com/superradcompany/microsandbox) — hardware-isolated microVM sandboxes
- [**TanStack DB**](https://tanstack.com/db) — reactive queries over materialized state

## License

Apache 2.0. See the [Apache 2.0 license](https://www.apache.org/licenses/LICENSE-2.0).
