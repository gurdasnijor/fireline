# Telegram

Telegram is the quickest way to turn a Fireline agent into something you can text.

On the shipped surface, Telegram is not a separate sidecar process. You compose `telegram(...)` into your spec, the host lowers that into a `TelegramSubscriber` durable-subscriber profile, and the same durable stream continues to power chat replies, approvals, and restart recovery.

## What This Does

Use Telegram when you want:

- a DM or group chat to become the prompt surface for an agent
- streamed replies to show up in the same chat
- approval buttons to live where the operator already is
- more than one agent to speak in the same chat without building a custom dashboard

The public building blocks on `main` today are:

- `telegram(...)` from `@fireline/client/middleware`
- `approve({ scope: 'tool_calls' })` when you want inline Approve / Deny cards
- `peer()` middleware when one agent should ask another agent to help
- W3C trace context carried through `_meta.traceparent`, `_meta.tracestate`, and `baggage`

## Before You Start

Create a bot with `@BotFather` first. The fastest setup is:

1. run `/newbot` in Telegram
2. copy the bot token
3. start from [deploy/telegram/bridge.env.example](../../deploy/telegram/bridge.env.example)

Example:

```bash
cp deploy/telegram/bridge.env.example deploy/telegram/bridge.env
$EDITOR deploy/telegram/bridge.env
set -a
source deploy/telegram/bridge.env
set +a
```

What the template gives you:

- `TELEGRAM_BOT_TOKEN` for the required bot credential
- optional `TELEGRAM_CHAT_ID` if you want to pin delivery to one chat
- optional `TELEGRAM_ALLOWED_USER_IDS` if you want to restrict who can talk to the bot
- `FIRELINE_URL` for the control plane
- `AGENT_COMMAND` if you do not want the demo to default to `pi-acp`

One honest detail: the checked-in `examples/telegram-demo/agent.ts` only requires `TELEGRAM_BOT_TOKEN`. If you want `chatId` or `allowedUserIds`, thread those values into your own `telegram(...)` call as shown below.

## Fastest Way To Try It

The most user-facing path on `main` is the CLI:

```bash
pnpm install
set -a
source deploy/telegram/bridge.env
set +a
npx fireline run examples/telegram-demo/agent.ts
```

Expected output excerpt:

```text
durable-streams ready at http://127.0.0.1:7474/v1/stream

  ✓ fireline ready

    ACP:       ws://127.0.0.1:...
    state:     http://127.0.0.1:7474/v1/stream/...
```

After that:

1. open Telegram and DM your bot
2. send `read README.md and summarize it in two sentences`
3. watch the reply stream back into the same chat

If you want to force an approval card, send a prompt such as:

```text
Delete /workspace/dist, but stop and ask me before you run the tool.
```

Expected behavior:

- the agent replies in Telegram
- the tool call pauses before it executes
- Telegram renders an inline `Approve` / `Deny` keyboard
- tapping a button resumes the same durable session

If you already have a Fireline host running and want the lower-level TypeScript control-plane path instead, the checked-in example still works directly:

```bash
cd examples/telegram-demo
pnpm install
set -a
source ../../deploy/telegram/bridge.env
set +a
pnpm start
```

## Minimal Compose Shape

For new code, prefer `scope: 'tool_calls'` on `telegram(...)`. The older `events: ['permission_request']` spelling is still accepted for compatibility, but `scope` is the clearer public surface.

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { approve, telegram, trace } from '@fireline/client/middleware'
import { localPath } from '@fireline/client/resources'

const allowedUserIds = (process.env.TELEGRAM_ALLOWED_USER_IDS ?? '')
  .split(',')
  .map((value) => value.trim())
  .filter(Boolean)

const spec = compose(
  sandbox({
    resources: [localPath(process.env.REPO_PATH ?? '../..', '/workspace')],
    labels: { demo: 'telegram-demo' },
  }),
  middleware([
    trace(),
    approve({ scope: 'tool_calls' }),
    telegram({
      token: { ref: 'env:TELEGRAM_BOT_TOKEN' },
      chatId: process.env.TELEGRAM_CHAT_ID,
      allowedUserIds,
      scope: 'tool_calls',
    }),
  ]),
  agent((process.env.AGENT_COMMAND ?? 'pi-acp').split(' ')),
)
```

That is the whole pattern:

- `trace()` keeps the chat interaction visible in your observability backend
- `approve(...)` makes risky tool calls wait for a decision
- `telegram(...)` turns that durable state into a chat UI

## Approvals In The Same Chat

Telegram is a good fit for approvals because the operator does not need to switch surfaces.

When `approve({ scope: 'tool_calls' })` and `telegram(...)` are in the same middleware array:

- the approval gate writes a durable `permission_request`
- Telegram renders that request as an inline card
- an Approve or Deny tap appends `approval_resolved`
- the original run resumes from the same session and request ids

If you want a replayable proof on `main`, use the public FQA-4 pack:

- [docs/demos/fqa-approval-demo-capture.md](../demos/fqa-approval-demo-capture.md)
- [docs/demos/scripts/fqa-approval-harness.ts](../demos/scripts/fqa-approval-harness.ts)
- [docs/demos/scripts/replay-fqa-approval.mjs](../demos/scripts/replay-fqa-approval.mjs)

That capture is useful even if your real UI is Telegram, because it proves the approval substrate separately from the chat transport.

## Peer Reviewers In The Same Chat

Telegram also works as a shared surface for more than one agent.

The pattern is:

- give the primary agent `telegram(...)`
- give the reviewer agent the same `telegram(...)` config
- put both agents on the same discovery surface
- enable `peer()` middleware where delegation should happen

From the operator's point of view, this means:

- the primary agent can answer in chat
- the reviewer can answer in the same chat
- the trace tree still shows which agent did what

For the durable peer-hop behavior itself, see:

- [docs/guide/multi-agent.md](./multi-agent.md)
- [docs/demos/peer-to-peer-demo-capture.md](../demos/peer-to-peer-demo-capture.md)

## Polling, Webhooks, And Keep-Alive

The current shipped Telegram surface uses Telegram polling settings on the subscriber profile:

- `pollIntervalMs` defaults to `1000`
- `pollTimeoutMs` defaults to `30000`
- `parseMode` defaults to `html`

What that means operationally:

- you do not need to run a separate webhook server just to try Telegram locally
- the host process needs outbound access to the Telegram Bot API
- if the host dies, chat delivery pauses until the process comes back and polling resumes

That is still a durable workflow: the session state lives in the stream, not in the poll loop. But the chat UI itself only updates while the subscriber is connected.

## What Could Go Wrong

- `telegram(...)` requires a bot token.
  Target-only routing is not supported by the live lowering; if you omit `token`, the client throws before provisioning.
- Long streamed replies can hit Telegram edit-rate limits.
  In practice, this means message edits may arrive in larger chunks than your local terminal output.
- Approval gating is still a little rough at the prompt boundary.
  The public approval replay on `main` still reports `promptLevelFallback: true`, so the durable behavior is correct but the operator-facing messaging is not yet as specific as the API name suggests.
- The demo example is intentionally minimal.
  If you need a fixed chat, allowlist, or non-default polling settings, pass `chatId`, `allowedUserIds`, `pollIntervalMs`, or `pollTimeoutMs` yourself.
- Older Telegram bridge docs describe a different shape.
  [examples/telegram-bridge/README.md](../../examples/telegram-bridge/README.md) is still useful as background for bot bootstrap and probing, but the surfaced demo path is `telegram(...)` middleware inside the Fireline spec.

## Deeper References

- [examples/telegram-demo/README.md](../../examples/telegram-demo/README.md)
- [docs/guide/approvals.md](./approvals.md)
- [docs/guide/durable-subscriber.md](./durable-subscriber.md)
- [docs/guide/multi-agent.md](./multi-agent.md)
- [docs/demos/pi-acp-to-openclaw-operator-script.md](../demos/pi-acp-to-openclaw-operator-script.md)
- [docs/proposals/durable-subscriber.md](../proposals/durable-subscriber.md) for the substrate-level design detail
