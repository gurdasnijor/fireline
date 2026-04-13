# Telegram Demo

You do not want another operator dashboard for simple agent work. You want to text the agent, see the reply in the same chat, and approve risky tool calls without leaving Telegram. Most stacks solve that by bolting a bot bridge onto a separate control plane.

This example shows the Fireline version of that flow. `telegram(...)` is composed directly into the spec, the host lowers it into the `TelegramSubscriber` durable-subscriber profile on `main`, and the same durable stream keeps the chat surface, approval state, and restart recovery in sync. No separate bridge process is the product story anymore.

## What This Example Shows

1. `telegram({ token: { ref: 'env:TELEGRAM_BOT_TOKEN' }, scope: 'tool_calls' })` turns Telegram into the chat surface for the agent.
2. `approve({ scope: 'tool_calls' })` makes risky tool calls pause and wait for an inline Telegram decision.
3. Optional `TELEGRAM_CHAT_ID` and `TELEGRAM_ALLOWED_USER_IDS` keep delivery scoped to the right chat and operator.
4. The script only provisions the spec. The durable-subscriber profile on the host keeps polling Telegram after the launch command exits.

## The Code

```ts
const spec = compose(
  sandbox({
    resources: [localPath(repoPath, '/workspace')],
    labels: { demo: 'telegram-demo', channel: 'telegram' },
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
  agent(agentCommand),
)
```

That is the post-pivot claim: Telegram is just another middleware entry in the Fireline topology, backed by the same durable-subscriber substrate as the rest of the runtime.

## Run It

The quickest path is the Fireline CLI:

```bash
cp deploy/telegram/bridge.env.example deploy/telegram/bridge.env
$EDITOR deploy/telegram/bridge.env
set -a
source deploy/telegram/bridge.env
set +a

npx fireline run examples/telegram-demo/agent.ts
```

By default the example uses `npx -y @agentclientprotocol/claude-agent-acp`. Override `AGENT_COMMAND` if you want a different ACP agent.

If you already have a Fireline host running and want the lower-level TypeScript provisioning path instead:

```bash
cd examples/telegram-demo
pnpm install --ignore-workspace --lockfile=false
set -a
source ../../deploy/telegram/bridge.env
set +a
pnpm start
```

The script prints the ACP URL and `stateStream` URL for the provisioned runtime. After that, talk to the bot in Telegram.

## Demo Prompt

A good first chat is:

```text
Read README.md and summarize what this repository does in two sentences.
```

To force the approval path, send:

```text
Delete /workspace/dist, but stop and ask me before you run the tool.
```

Expected behavior:

- the reply streams into the Telegram chat
- the risky tool call pauses before it runs
- Telegram renders an inline Approve / Deny action
- tapping a button resumes the same durable session

## Preflight

`pnpm smoke` is the lightweight example check. It always validates the compose surface and, when `TELEGRAM_BOT_TOKEN` is present, also calls Telegram `getMe` to confirm the bot token is live.

For the older bridge bootstrap and probing story, use [telegram-bridge](../telegram-bridge/README.md) only as background reference. It is no longer the front-door example.
