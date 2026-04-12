# Demo Plan

This document proposes five demo scenarios that show Fireline's actual advantage, not just its architecture. Each demo is framed around what the audience sees, why it matters, what features make it possible, and how close it is to something we can run live.

## Demo 1 — The Unkillable Agent

**Story**

An agent is in the middle of a real task: reading files, writing code, and streaming a response back to the UI. Halfway through a sentence, the demo operator kills the process. The UI freezes for a moment, a new runtime is launched on a different machine, and then the exact same conversation continues from the same session instead of starting over. The audience sees the response resume as if the process crash was just a network hiccup.

**Why it's impressive**

Most agent systems treat the running process as the session. When the process dies, the session dies with it or gets reconstructed from a lossy summary. Fireline's story is stronger: the process is disposable, the session is not. That is a sharp, understandable difference from raw model APIs, LangChain-style orchestration, and the typical "please retry" failure mode people are used to.

**Implementation**

- Durable streams hold the source-of-truth session history.
- The runtime can be rebuilt from the saved session stream instead of relying on in-memory state.
- `session/load` rehydrates the session and resumes work from the stored evidence.
- The state read side can replay the stream and reconstruct the live view after failover.

**Feasibility**

High. This is the most native Fireline demo because it follows the system's core design rather than needing extra product glue. The missing work is mostly demo choreography: a clean operator script, a visible "old host / new host" display, and a controlled restart path that looks intentional on stage.

## Demo 2 — The Approval Gate

**Story**

An agent is reviewing a pull request and decides it wants to delete a risky file. Instead of acting immediately, it pauses and emits an approval request. A Slack message appears for a product manager or engineer. They approve it from their phone. The agent wakes back up and completes the change. The operator can also show the opposite path: deny the request and watch the agent adapt.

**Why it's impressive**

The important point is not just "human approval exists." Many products can fake that with a modal and a blocked process. Fireline's advantage is that approval is durable and asynchronous. The agent can wait for minutes or hours, the process can restart, and the approval still lands as part of the same live workflow. That makes it feel like infrastructure rather than a frontend trick.

**Implementation**

- `approve()` pauses risky actions before they execute.
- The approval request is written to the durable stream and becomes visible to observers.
- `@fireline/state` can drive the UI or webhook worker that surfaces the approval externally.
- Slack or webhook integration resolves the approval by writing the decision back into the system.
- The resumed agent continues from the same session after the approval event arrives.

**Feasibility**

Medium-high. The core approval path exists. The likely missing work is product glue: a polished Slack integration, a clean approval UI, and a stage-safe scenario with a deterministic "risky edit" action. This should be treated as a demo-focused integration sprint, not a platform invention sprint.

## Demo 3 — The Live Dashboard

**Story**

Fifty agents are running in parallel on background tasks. A dashboard shows all of them live: active runs, tool calls in flight, approvals waiting on humans, and completed work. Nothing refreshes manually. A few agents finish, one asks for approval, another crashes and comes back, and the dashboard just keeps moving in real time.

**Why it's impressive**

This demonstrates that Fireline is not only about running one clever agent. It is a control room for many agents at once. Most teams reach for custom polling, ad hoc logs, or one-off WebSocket code to get here. Fireline's pitch is that the live operational view falls out of the same data the system already needs to stay durable.

**Implementation**

- `@fireline/state` subscribes to the shared event stream.
- TanStack DB powers reactive queries over live agent activity.
- The same stream carries prompts, tool calls, approvals, and completion signals.
- The UI is just a query client over the stream-backed state rather than a custom event protocol.

**Feasibility**

High for a scoped dashboard, medium for a polished "50 agents" story. The plumbing exists. The main missing pieces are front-end polish, synthetic workload generation, and making sure the screen tells a clear story instead of turning into noise. This is very demoable if the UI is opinionated and selective.

## Demo 4 — Flamecast in 200 Lines

**Story**

Show a real agent management UI, not a toy. The audience sees the Flamecast client doing session launch, live state, approvals, and runtime control on top of Fireline. Then the presenter shows the code side-by-side: Flamecast's old custom infrastructure versus the Fireline-backed version. The message is simple: same product experience, dramatically less custom plumbing.

**Why it's impressive**

This turns Fireline from a systems pitch into a leverage pitch. Audiences trust proof points more than diagrams. If a recognizable product workflow can sit on Fireline without losing capabilities, the platform feels real. It also answers the common objection that infrastructure like this is "powerful but abstract."

**Implementation**

- The Fireline client provides runtime start/stop and connection handles.
- ACP carries the actual session traffic.
- `@fireline/state` supplies the live session and runtime view for the UI.
- Approval and trace middleware provide the operational controls the UI surfaces.
- The `examples/flamecast-client/` app is the natural vehicle for the demo.

**Feasibility**

Medium. This is credible, but it depends on the Flamecast-on-Fireline path being stable enough to show as a product story rather than a half-migrated prototype. It is a strong late demo, not the first demo we should depend on.

## Demo 5 — Provider Swap

**Story**

The presenter shows one agent definition. First it runs locally. Then the same setup runs in Docker. Then the same setup runs on Anthropic's managed cloud. The code shown to the audience stays the same except for provider selection. The visible behavior is the same session model, same controls, same observation flow.

**Why it's impressive**

This says Fireline is not tied to one runtime environment or one vendor. Anthropic's managed agents are compelling because they are easy. Fireline's answer is flexibility without fragmenting the programming model. If we can show one workflow moving across local, container, and hosted execution, that is a strong differentiation story.

**Implementation**

- The provider abstraction chooses how the agent is provisioned.
- Local subprocess and Docker-style providers prove self-hosted portability.
- A remote Anthropic-backed provider would prove that Fireline can sit above a managed agent platform instead of competing only at the same layer.
- The same client API should provision the agent and return the same control and observation surfaces.

**Feasibility**

Low-medium today. The story is strategically important, but it depends on provider work that is not yet complete, especially the Anthropic-backed path. This should be treated as a roadmap demo. It is excellent for vision decks and future launch plans, but risky as a near-term live demo unless the provider work lands cleanly.

## Recommended demo order

If we need a sequence for a live presentation, the safest order is:

1. The Unkillable Agent
2. The Approval Gate
3. The Live Dashboard
4. Flamecast in 200 Lines
5. Provider Swap

That order starts with the clearest "only Fireline can do this" moment, moves into operational control and visibility, then ends with product leverage and long-term portability.

## Recommendation

For the next demo cycle, prioritize three things:

- Build Demo 1 until it is stage-perfect. This is the signature story.
- Build Demo 2 with a real Slack approval path. This makes the platform feel operational.
- Build Demo 3 as the persistent background backdrop for the other demos. It reinforces that Fireline is a system, not a one-off trick.

Demo 4 should come online when the Flamecast example is genuinely persuasive. Demo 5 should stay in the roadmap narrative until the provider story is real enough to trust live.
