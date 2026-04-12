# Fireline

## What is Fireline?

Fireline is infrastructure for AI agents that gives your product three things most teams end up building badly by hand: live visibility, hard control points, and durable execution. It lets you run an agent, watch what it is doing, pause risky actions for approval, and keep the work alive even if the process or machine dies.

## The three things it gives you

### Visibility

Fireline turns agent activity into live product data. Prompts, tool calls, approvals, and results are all visible while they happen, not only after the fact in logs. That means you can build a dashboard that shows what agents are doing right now, alert when something risky happens, and give support or operations teams a real view into the system.

### Control

Fireline gives you control points around the agent that the agent cannot ignore. You can require human approval before certain actions, cap token spend, inject context, and route work to other agents or services. These rules live in infrastructure, not in the prompt, so they are enforceable and auditable.

### Durability

Fireline keeps the session history outside the running agent process. If the process crashes, the host restarts, or the work moves to another machine, the session can continue from the saved record instead of starting over. This is the difference between a toy agent and something you can trust for long-running work.

## How it compares to alternatives

Anthropic's managed agents are the easiest way to get started if you want a hosted Claude-native product and do not want to run infrastructure. Fireline is the better fit when you want the same kind of control and visibility but need self-hosting, multiple runtime options, or the freedom to build your own workflow and UI on top.

LangChain and similar frameworks help you write agent logic. Fireline helps you run agents as a system: start them, watch them, govern them, and recover them when things fail. They solve different problems and can be used together.

Raw API calls are the fastest path to a demo, but they leave you owning everything once the agent needs tools, approvals, dashboards, or crash recovery. Fireline exists for the moment when "call the model and hope" stops being enough.

## The compose model

Fireline's core setup model is one call: choose the sandbox, choose the control rules, choose the agent, then start it.

```typescript
import { compose, agent, sandbox, middleware } from '@fireline/client'
import { trace, approve, budget } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const handle = await compose(
  sandbox({ resources: [localPath('.', '/workspace')] }),
  middleware([trace(), approve({ scope: 'tool_calls' }), budget({ tokens: 500_000 })]),
  agent(['claude-code-acp']),
).start({ serverUrl: 'http://localhost:4440' })
```

That one handle gives your app one URL for talking to the agent and one URL for watching live state.

## Who uses it

Flamecast is the clearest proof point. The `examples/flamecast-client/` app shows a real agent management UI running on Fireline instead of on a large pile of custom runtime plumbing. That matters because it shows Fireline is not just an architecture idea; it is useful enough to replace real product code.

## What's coming

Cross-host discovery will let Fireline instances find each other automatically, so agents and workloads can move across machines without manual wiring. Resource discovery will make shared tools, files, and services visible through the same system instead of being passed around out of band. Stream-fs is the long-term file story: a shared file layer that follows the agent across hosts the same way session state already does.
