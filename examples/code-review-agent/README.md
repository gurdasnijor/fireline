# Code Review Agent

Can an AI review my code and still stop before it changes anything important? That is the real product question behind most "AI coding assistant" demos. People do not just want a faster bot. They want a reviewer that can move at machine speed without quietly rewriting files behind their back.

This demo shows the Fireline version of that story. You point an ACP-speaking coding agent at a real Git repo. It reads the code, proposes fixes, and when it reaches a file write Fireline turns that moment into a durable approval request instead of a hidden side effect. You can watch the run live because the state stream is the source of truth, not a pile of in-memory callbacks.

## What Happens

1. The agent gets mounted into a repo at `/workspace`.
2. `approve({ scope: 'tool_calls' })` makes every dangerous tool call pausable.
3. The script prints the `stateStream` URL you can open in the `live-monitoring` demo.
4. The first pending approval becomes a durable record in `@fireline/state`.

## The Code

```ts
const handle = await compose(
  sandbox({ resources: [localPath(repoPath, '/workspace')] }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    secretsProxy({
      ANTHROPIC_API_KEY: { ref: 'env:ANTHROPIC_API_KEY' },
    }),
  ]),
  agent(agentCommand),
).start({ serverUrl, name: 'code-review-agent' })
```

That one line is the behavior contract: run a real coding agent in a real repo, but make file-changing tool calls human-gated and observable.

## Run It

```bash
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/code-review-agent
pnpm install
REPO_PATH=/path/to/git/repo \
ANTHROPIC_API_KEY=... \
pnpm start
```

The script prints a `stateStream` URL. Point `examples/live-monitoring` at that URL and you get the product experience a buyer actually cares about: a code-review agent they can trust.

## The Primitive Behind This Example

The conceptual foundation for this example is the passive durable workflow
pattern described in [durable-subscriber.md](../../docs/proposals/durable-subscriber.md).

In the target architecture, a tool-call approval flow like this is modeled as a
`DurableSubscriber::Passive` wait keyed by the canonical ACP prompt-request
reference `(SessionId, RequestId)`, not by a Fireline-minted review id. The
approval request is durable because it is written to the session stream, and the
resolution path is durable because the matching completion is appended back to
that same stream.

[acp-canonical-identifiers.md](../../docs/proposals/acp-canonical-identifiers.md)
is the other half of the story: it defines why the stable identity for this
approval is the ACP prompt request itself. This README is describing the product
shape that substrate enables, not claiming the fully generalized
`DurableSubscriber` runtime is already the implementation behind this example
today.
