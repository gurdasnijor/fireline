# Code Review Agent

The buyer question is not "can an AI look at my repo?" It is "can it review a
real diff at machine speed without quietly changing anything?"

This example is the Fireline answer:

- mount the repo into the sandbox read-only
- run a review prompt against the real workspace
- read the review text back from the durable state stream

That gives you a trustworthy review surface first. If you later want the agent
to draft patches, you can add the approval workflow on top of the same
infrastructure.

## What This Example Shows

1. The repo is mounted into `/workspace` as a read-only resource.
2. A coding agent reviews the mounted repo as if it were a PR.
3. Fireline records the run to a durable state stream.
4. The script prints the review text plus the `stateStream` URL you can open in
   `examples/live-monitoring`.

## The Code

```ts
const handle = await compose(
  sandbox({
    provider: 'local',
    resources: [localPath(repoPath, '/workspace', true)],
    envVars: process.env.ANTHROPIC_API_KEY
      ? { ANTHROPIC_API_KEY: process.env.ANTHROPIC_API_KEY }
      : undefined,
  }),
  middleware([trace()]),
  agent(agentCommand),
).start({ serverUrl, name: 'code-review-agent' })
```

The important detail is the resource mount:

- the agent sees the real repo at `/workspace`
- the mount is read-only, so review cannot silently turn into writes
- the state stream still records the whole run for dashboards and later audit

## Run It

Prerequisites:

- a Fireline host reachable at `FIRELINE_URL` or `http://127.0.0.1:4440`
- Node `>=20`
- `pnpm`
- either:
  - `ANTHROPIC_API_KEY` for the default `claude-agent-acp` path, or
  - a local ACP test agent via `AGENT_COMMAND`

Install dependencies:

```bash
pnpm install
cd examples/code-review-agent
pnpm install --ignore-workspace --lockfile=false
```

Fast deterministic smoke on current `main`:

```bash
AGENT_COMMAND=/absolute/path/to/fireline/target/debug/fireline-testy-prompt \
pnpm start
```

The scripted agent just echoes the review prompt, which makes the run stable
and testable.

Default product-shaped run with Claude Agent ACP:

```bash
REPO_PATH=/path/to/git/repo \
ANTHROPIC_API_KEY=... \
pnpm start
```

The script prints the `stateStream` URL. Point `examples/live-monitoring` at
that URL and you get the product experience a buyer actually cares about: a
review agent inspecting a real repo with a durable audit trail.

## Why This Is The Right Story

This example is not trying to be "AI writes code." It is the safer first step:

- hand the agent a real repo
- keep the mount read-only
- get a review you can inspect, store, and replay later

That is a more compelling product story than a toy diff parser because it maps
to the real adoption path teams take: review first, patching later.

## Honest Notes On Current `main`

- This example avoids `secretsProxy()`.
  `mono-4t4` is still open, so the current safe path for the demo is direct
  `sandbox({ envVars })` or a local test-agent override.
- The deterministic smoke path uses `fireline-testy-prompt`.
  That proves the repo-mount plus state-stream readback path. The Claude path
  is the one that turns this into a real review workflow.
- The repo mount is intentionally read-only.
  If you want the agent to propose edits and wait for approval before applying
  them, pair this example with [approval-workflow](../approval-workflow/README.md).

## Read Next

- [Approval Workflow](../approval-workflow/README.md)
- [Observation](../../docs/guide/observation.md)
- [Resources](../../docs/guide/resources.md)
