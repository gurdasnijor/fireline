# Fireline — Pitch Deck

*15-minute demo presentation. 10 slides. Non-technical audience.*

---

## Slide 1: What if your AI agent could never lose its work?

You give an agent a task. It works for an hour. The process crashes. Today, you lose everything and start over. With Fireline, the agent picks up exactly where it left off — on any machine, after any failure.

**Fireline is infrastructure that makes AI agents durable, visible, and controllable.**

[DIAGRAM: An agent working across a timeline. A red "crash" icon in the middle. The agent continues seamlessly on the other side. Caption: "The session survives everything."]

---

## Slide 2: The Problem

Every company building AI features hits the same three walls:

- **Black box.** The agent is running commands, calling APIs, making decisions. You find out what happened after the fact — from logs, if you're lucky. While it's working, you're blind.
- **No kill switch.** The agent has a shell, API keys, access to production data. If it decides to do something dangerous, there's nothing between the decision and the action. Prompt-level guardrails are suggestions the agent can ignore.
- **Fragile.** A 30-minute task crashes at minute 25. The conversation, the tool calls, the partial results — all gone. The user sees an error and gives up.

These aren't edge cases. They're the daily reality of running agents in production.

[DIAGRAM: Three panels showing the pain. Panel 1: a blank screen with "What is the agent doing?" Panel 2: a terminal running `rm -rf` with no gate. Panel 3: a progress bar at 95% with a crash error.]

---

## Slide 3: Fireline Makes Agents Visible

Every action the agent takes — every prompt, every tool call, every response, every decision — is recorded to a live event stream in real time. Not as logs. As structured, queryable data.

- **One dashboard for all your agents.** 50 agents running in parallel? One screen shows all of them — active sessions, pending approvals, completion status.
- **Real-time, not polling.** Your UI subscribes to the stream and updates automatically. No 5-second refresh loops. No custom WebSocket code.
- **The code is trivial.** Ten lines replace hundreds.

```typescript
const db = createFirelineDB({ stateStreamUrl: handle.state.url })

const turns = useLiveQuery(q =>
  q.from({ t: db.collections.promptTurns })
)
```

[DIAGRAM: A live dashboard showing agent activity — sessions listed on the left, a conversation trace in the center, pending approvals highlighted in amber. Reference: `assets/before-after.svg`]

---

## Slide 4: Fireline Makes Agents Controllable

Every interaction between your application and the agent passes through a middleware pipeline. Each middleware is a rule — not a prompt, not a suggestion, but an infrastructure-level gate that the agent cannot bypass.

- **Approval gates.** The agent wants to run a shell command? It pauses. A notification appears in your dashboard (or Slack, or email). A human reviews and approves or denies. One line of config.
- **Budget caps.** Cap an agent at 500,000 tokens. The infrastructure enforces it — the agent can't spend more no matter what it tries. One line of config.
- **Credential isolation.** The agent needs API keys to do useful work, but if it can see the key it can leak it. Fireline injects credentials at call time without ever exposing them to the agent. One line of config.

```typescript
middleware([
  trace(),
  approve({ scope: 'tool_calls' }),
  budget({ tokens: 500_000 }),
  secretsProxy({ GITHUB_TOKEN: { ref: 'secret:gh-pat', allow: 'api.github.com' } }),
])
```

[DIAGRAM: The middleware pipeline — four boxes in a row (trace, approve, budget, inject), each intercepting messages between your app and the agent. Reference: `assets/middleware-pipeline.svg`]

---

## Slide 5: Fireline Makes Agents Durable

The agent's entire history — every prompt, every response, every tool call, every approval — lives in a durable event stream, not in the agent process. If the process crashes, the work is still there. If the host fails, the work is still there.

- **Kill it, restart it anywhere.** Stop the agent on your laptop. Start it on a cloud server. It continues from exactly where it left off. Not "here's a summary" — the actual conversation, replayed event by event.
- **Approvals outlive the sandbox.** The agent requests permission. You close your laptop. The sandbox times out and dies. Five hours later, you approve from your phone. A new sandbox provisions, replays the stream, and the agent continues.
- **Your 3am batch job is safe.** A container recycles at hour 4 of a 5-hour job. The orchestrator detects it, provisions a new sandbox, and the agent resumes from its last checkpoint. The morning report shows a blip, not a failure.

[DIAGRAM: Timeline showing an agent working, the sandbox dying, hours passing with no process running, a user approving from their phone, and the agent resuming on a fresh sandbox. Reference: `assets/durable-wait.svg`]

---

## Slide 6: How It Works

Fireline separates agent infrastructure into three clean layers:

| Layer | What it does | Who uses it |
|---|---|---|
| **Control** | Provision sandboxes. Wire middleware. Start agents. | Your backend / DevOps |
| **Session** | Send prompts. Receive responses. Open conversations. | Your application code |
| **Observation** | Subscribe to live agent activity. Query state reactively. | Your dashboard / monitoring |

Each layer has its own endpoint. No tangled dependencies. No side channels.

The entire setup is a single function call:

```typescript
const handle = await compose(
  sandbox({ resources: [...] }),
  middleware([trace(), approve(), budget()]),
  agent(['claude-code-acp']),
).start({ serverUrl })

// handle.acp   → Session plane (open conversations, send prompts)
// handle.state → Observation plane (subscribe to live activity)
```

[DIAGRAM: Three columns — Control (blue), Session (purple), Observation (green) — each with its package name and key API. Arrows from a central handle connecting to each. Reference: `assets/three-planes.svg`]

---

## Slide 7: Real Leverage

We measured this against a real product. Flamecast — our AI agent platform — had 5,300 lines of hand-built agent infrastructure: WebSocket management, REST polling fallbacks, status merging, session tracking, approval flows, error recovery.

With Fireline, that collapses to under 200 lines.

| Component | Before Fireline | With Fireline |
|---|---|---|
| Real-time agent observation | 186 lines (WebSocket + REST polling + merge logic) | 10 lines (`useLiveQuery`) |
| Approval flow | ~400 lines (custom WebSocket events + state machine) | 1 line (`approve({ scope: 'tool_calls' })`) |
| Session durability | ~800 lines (checkpoint/restore + error recovery) | 0 lines (built into the stream) |
| Budget enforcement | ~200 lines (token counting + limit checks) | 1 line (`budget({ tokens: 500_000 })`) |
| Multi-agent coordination | ~1,200 lines (custom orchestrator) | 1 line (`peer(reviewer, writer)`) |
| **Total infrastructure** | **~5,300 lines** | **~200 lines** |

That's not a marginal improvement. That's deleting an entire layer of your stack.

[DIAGRAM: Side-by-side code comparison. Left: 186 lines of WebSocket infrastructure (greyed out, scrolling). Right: 10 lines of useLiveQuery (highlighted green). Caption: "-176 lines". Reference: `assets/before-after.svg`]

---

## Slide 8: Provider Portable

The same `compose()` call works everywhere. Change the provider — keep everything else.

| Environment | What changes | What stays the same |
|---|---|---|
| **Local dev** | `sandbox()` runs a subprocess | Middleware, agent, observation |
| **Docker** | `sandbox({ provider: 'docker' })` | Middleware, agent, observation |
| **MicroVM** | `sandbox({ provider: 'microsandbox' })` | Middleware, agent, observation |
| **Cloud** | `serverUrl: 'https://prod.fireline.dev'` | Middleware, agent, observation |

Your staging environment and production environment run the same code. The only difference is where the sandbox runs and which stream it writes to.

This means:
- **Dev/prod parity.** Test locally with full middleware. Deploy to cloud with zero config changes.
- **No vendor lock.** Start with local subprocesses today. Move to hardware-isolated VMs when you need security. Move to Anthropic's managed agents when you want zero-ops. Your application code doesn't change.

[DIAGRAM: Four identical compose() blocks, each pointing to a different provider icon (laptop, Docker whale, microVM chip, cloud). Same middleware in all four. Caption: "Write once, run anywhere."]

---

## Slide 9: What's Coming

Fireline is open source (Apache 2.0) and actively developed. The next phase:

- **Cross-host discovery.** Agents on different machines find each other automatically through the durable stream. No service registry. No DNS. The stream IS the discovery mechanism.
- **Resource discovery.** An agent publishes a file or dataset to the stream. Other agents discover and mount it — across hosts, across providers, with access control and audit.
- **Stream-FS.** A collaborative filesystem backed by the durable stream. Multiple agents working on the same codebase, with every change tracked and replayable.
- **Formal verification.** The core state machines are already modeled in TLA+ and checked on every commit. As the system grows, the verification grows with it.

[DIAGRAM: A network graph showing agents on different hosts, connected by durable stream lines, with resources flowing between them. Caption: "Agents that find each other."]

---

## Slide 10: Get Started

Three lines. That's the entire setup.

```
npm install @fireline/client
```

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve, budget } from '@fireline/client/middleware'

const handle = await compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([trace(), approve({ scope: 'tool_calls' }), budget({ tokens: 500_000 })]),
  agent(['claude-code-acp']),
).start({ serverUrl: 'http://localhost:4440' })
```

- **Open source.** Apache 2.0. Self-host, fork, contribute.
- **Production-ready core.** Durable streams, middleware pipeline, session management — all formally verified.
- **Works with any model.** Claude, GPT, Gemini, open-source. Fireline is the infrastructure around the model call, not the model call itself.

[DIAGRAM: The Fireline logo. Below it: GitHub stars count, "Apache 2.0", and the repo URL.]

**GitHub:** github.com/anthropics/fireline

---

*Built on [durable-streams](https://durablestreams.com), [ACP](https://agentclientprotocol.com), and [TanStack DB](https://tanstack.com/db).*
