# User Surfaces

> Related:
> - [`vision.md`](./vision.md)
> - [`object-model.md`](./object-model.md)
> - [`ecosystem-story.md`](./ecosystem-story.md)
> - [`../runtime/agent-catalog-and-launch.md`](../runtime/agent-catalog-and-launch.md)
> - [`../ts/primitives.md`](../ts/primitives.md)

## What End Users Should Actually Do

At a high level, an end user flow should look like this:

1. Connect or choose a workspace.
2. Pick an agent or run template.
3. Pick a capability profile.
4. Choose execution placement:
   - local
   - remote
   - auto
5. Start a run, which creates or resumes a session.
6. Observe progress live, intervene if needed, and reopen it later from another
   device or product surface.

That is a much better mental model than asking users to think directly about
conductors, runtimes, and durable streams.

## Example End-User Flows

### Personal coding session

The user:

- selects a local repo as a workspace
- chooses a coding profile with GitHub, shell, and docs MCPs
- starts locally
- later moves execution to a remote runtime without losing the session
- reopens the same session from their phone or browser

Under the hood, Fireline is:

- creating a session
- binding that session to a workspace
- placing execution on a runtime
- projecting events into durable state
- preserving lineage and transcript for later reopen

### Background repo maintenance

The user:

- connects a repo
- chooses a maintenance or CI-fix profile
- schedules a run or triggers it from a webhook
- lets it continue after the initiating client disconnects
- reviews the result later with full audit trail

This is one of the strongest demonstrations of the "session outside the
harness" model.

### Team-shared specialist agents

The user:

- creates several capability profiles such as reviewer, docs writer, and
  release helper
- routes work to different runtimes or peers
- keeps one durable session graph tying the handoffs together
- inspects child sessions, approvals, and outputs from a shared control surface

This is where Fireline's lineage-aware peer model becomes product-visible.

### Embedded agent in another product

The end user may never know Fireline exists.

They click:

- "Fix this PR"
- "Investigate this alert"
- "Draft a customer reply"

The host product supplies:

- the workspace or source artifact
- the capability profile
- the runtime placement policy

Fireline supplies:

- durable run/session semantics
- reusable extension components
- replayable state and audit trail

## Likely Product Surfaces

Fireline should be able to show up through multiple product shapes, not just a
single app.

### 1. Editor or CLI companion

Examples:

- VS Code / Zed / JetBrains integration
- terminal-first CLI

What users should get:

- start or resume sessions from the current workspace
- attach profiles and policies to runs
- move long-running work off-machine
- inspect session history and child runs

### 2. Browser control plane

Examples:

- a hosted dashboard
- a self-hosted operator UI
- a lightweight mobile-friendly session viewer

What users should get:

- list sessions and runs
- inspect current and past state
- approve or deny gated actions
- reattach to or resume work
- observe multi-agent progress without connecting directly to every runtime

### 3. Workflow backend

Examples:

- Slackbot backend
- GitHub/CI automation
- support-ticket workflow
- incident-response assistant

What users should get:

- a durable run created from an event
- policy and audit around the run
- resumability and observability without a foreground client

### 4. Embedded platform primitive

Examples:

- internal developer portal
- customer support platform
- data or ops product with an agent feature

What product teams should get:

- one durable run object they can store in their own records
- one session transcript they can inspect later
- one set of extension and policy mechanisms reusable across many agent
  experiences

## Where Fireline Fits Into Existing Workflows

The point is not to replace every existing workflow. It is to sit underneath
them where durable agent state and reusable extensions matter.

### Existing coding workflows

Fireline can slot under:

- local coding copilots
- "run an agent on this repo" flows
- background code migration or review jobs
- long-running fix/build/test loops

Key value:

- persistent sessions
- resumability
- auditability
- move between local and remote execution without changing the logical run

### Existing workflow tools

Fireline can slot under:

- Slack-triggered jobs
- GitHub Actions / PR bots
- support ticket automation
- scheduled maintenance or research runs

Key value:

- every workflow invocation becomes a durable session
- the run can outlive the webhook request or queue worker
- operators can inspect and intervene later

### Existing extension ecosystems

Fireline can slot under:

- MCP tool ecosystems
- instruction-file ecosystems
- editor plugin ecosystems

Key value:

- unify extension behavior behind conductor components and capability profiles
- avoid reimplementing the same concern for each agent and editor separately

## A More Product-Like API Direction

The systems primitives should remain low-level.

But if Fireline wants to feel integrated into real products, a higher-level API
likely looks more like:

```ts
const workspace = await client.workspaces.connectLocal({
  path: "/Users/me/repo",
})

const profile = await client.profiles.get("coding-default")

const run = await client.runs.start({
  workspace,
  profile,
  placement: { mode: "auto" },
  agent: { source: "catalog", agentId: "codex-acp" },
})

const session = await client.sessions.open(run.sessionId)
```

Or:

```ts
const run = await client.runs.startFromWebhook({
  event: githubPullRequestOpened,
  workspace: { kind: "git", repoUrl, ref },
  profile: "pr-reviewer",
})
```

This is not a committed API.

It is a signal that the right product surface is probably:

- session-centric
- workspace-aware
- capability-profile-aware
- placement-aware

with the current primitives still serving as the substrate underneath.
