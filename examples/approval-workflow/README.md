# Approval Workflow

The product question is simple: how do you let an agent move fast without giving it silent permission to do dangerous things? "Trust the model" is not a workflow. It is an outage postmortem waiting to happen.

This demo shows the Fireline answer. The agent reaches for a risky operation, Fireline pauses the run, emits a durable approval record, and an external workflow gets a chance to decide. That workflow can be a webhook, Slack action, email link, or any other human system. When someone approves, the agent continues from the same turn and the decision stays in the audit trail.

## What Happens

1. `approve({ scope: 'tool_calls' })` turns dangerous tool calls into approval checkpoints.
2. A tiny webhook server receives the approval request.
3. The approval decision is appended back into the same state stream.
4. The original prompt resumes instead of restarting.

## The Code

```ts
const handle = await compose(
  sandbox({}),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    secretsProxy({ ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' } }),
  ]),
  agent(agentCommand),
).start({ serverUrl, name: 'approval-workflow' })
```

That is the product surface. The rest of the demo is ordinary glue around the durable approval event: send a notification out, receive a decision back, append the resolution, continue the run.

## Run It

```bash
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/approval-workflow
pnpm install
ANTHROPIC_API_KEY=... \
pnpm start
```

The output shows the request id, the approval outcome, and the same prompt turn completing after the external decision comes back.
