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

**This is the north-star file: one ACP agent, wrapped with tracing, approvals, budgets, secrets, and peer discovery.**

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { approve, budget, peer, secretsProxy, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

export default compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    budget({ tokens: 2_000_000 }),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
      GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' },
    }),
    peer({ peers: ['reviewer'] }),
  ]),
  agent(['pi-acp']),
)
```

**Run it locally. Build it for deployment. Same spec.**

```bash
npx fireline run agent.ts
#   ✓ fireline ready
#     sandbox: runtime:59f5ed5a-d624-…
#     ACP:     ws://127.0.0.1:54896/acp
#     state:   http://127.0.0.1:7474/v1/stream/fireline-state-runtime-…

npx fireline build agent.ts --target fly
#   ✓ fireline build complete
#     image:     fireline-agent:latest
#     scaffold:  /path/to/fly.toml
```

This is the shape behind the [pi-acp → OpenClaw](docs/demos/pi-acp-to-openclaw.md) story: start with a local agent, then keep the same authored spec as you add operator-facing visibility, approval gates, and peer workflows.

## What You Stop Hand-Building

<picture>
  <img alt="Before and after Fireline" src="assets/before-after.svg" width="100%">
</picture>

Fireline replaces the usual pile of custom agent plumbing: polling loops, approval callbacks, status merging, restart glue, session tracking, and one-off dashboards.

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
      Inject credentials at call time without handing plaintext tokens to the agent, then audit the use through the same durable surface.
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

```bash
npm install @fireline/client @fireline/cli
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

If you are running from this repo checkout, the CLI still resolves the local Rust binaries described in the [CLI guide](docs/guide/cli.md). A shorter guided quick start is landing at `docs/guide/quickstart.md`; until then, start with the [Developer Guide](docs/guide/README.md), [CLI guide](docs/guide/cli.md), and [Compose and Start](docs/guide/compose-and-start.md).

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
