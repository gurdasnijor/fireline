# Telegram Demo

This is the TypeScript demo spec for the Telegram pivot: `telegram()` is composed into the middleware array, and the Rust host turns that into the `TelegramSubscriber` DurableSubscriber profile landed by `mono-axr.11`.

This directory stays local until `mono-axr.6` lands the real `telegram()` middleware lowering on `main`. The Rust profile is already live; this example is the compose-surface draft that sits on top of it.

## What This Example Is For

- show the 15-line `compose(...)` file used in Demo Step 3
- keep Telegram in the same substrate as `trace()` and `approve()`
- avoid the old separate `examples/telegram-bridge` process as the demo path

The runtime story this file targets is:

1. start the agent with `telegram({ token: { ref: 'env:TELEGRAM_BOT_TOKEN' }, events: ['permission_request'] })`
2. DM the bot on Telegram
3. Fireline turns the inbound message into the session prompt path
4. a tool-call approval becomes an inline Approve / Deny card in Telegram
5. tap a button and the same session resumes in the same chat

## Prereqs

- `TELEGRAM_BOT_TOKEN` exported from `deploy/telegram/bridge.env`
- Fireline host reachable at `FIRELINE_URL` or `http://127.0.0.1:4440`
- `pi-acp` installed on `PATH`, or `AGENT_COMMAND` set to another ACP agent
- optional `REPO_PATH` if you do not want the repo root mounted at `/workspace`

If you need the older bot bootstrap and health probe patterns, use
[telegram-bridge](../telegram-bridge/README.md) as reference only. It is not
the demo path anymore.

## The Spec

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, telegram, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const repoPath = process.env.REPO_PATH ?? '../..'
const agentCommand = (process.env.AGENT_COMMAND ?? 'pi-acp').split(' ')

export default compose(
  sandbox({ resources: [localPath(repoPath, '/workspace')], labels: { demo: 'telegram-demo' } }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    telegram({ token: { ref: 'env:TELEGRAM_BOT_TOKEN' }, events: ['permission_request'] }),
  ]),
  agent(agentCommand),
)
```

That is the whole point of the pivot: Telegram is just another middleware entry
that lowers into a DurableSubscriber profile inside the host.

One current surface detail: the merged TypeScript API on `main` models
TelegramSubscriber matching through `events`, not the earlier draft
`scope: 'tool_calls'` shorthand from the operator script.

## Run

```bash
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/telegram-demo
pnpm install
set -a
source ../../deploy/telegram/bridge.env
set +a
pnpm start
```

The script prints the Fireline `stateStream` URL once the harness starts.

## Demo Flow

Send the bot a DM that forces a gated tool call, for example:

```text
Delete /workspace/dist, but stop and ask me before you run the tool.
```

Expected behavior once the full Step 3 and Step 4 demo path is on `main`:

- the first reply streams back into the Telegram chat as message edits
- the approval gate renders an inline keyboard card in the same chat
- tapping Approve or Deny appends the durable completion and unblocks the run

## Optional Smoke

`pnpm smoke` is a lightweight preflight for the compose spec. It validates the
Telegram token with `getMe` and checks that the example still carries
`trace`, `approve`, and `telegram` middleware in the expected order.
