# Telegram Bridge

This example is the `.6.3.2` foundation for the pi-acp -> OpenClaw Telegram
signature moment. It intentionally does **not** implement ACP prompt routing,
streaming message edits, session mapping, or approval cards yet. Later beads
add those behaviors on top of this harness.

What it does today:

- boots Chat SDK with the Telegram adapter in polling mode
- verifies the bot token with Telegram `getMe`
- probes Fireline host and durable-streams `/healthz`
- exposes `GET /healthz`
- optionally posts a startup message to `TELEGRAM_CHAT_ID` so the bot is visibly
  online in the demo chat

## Run

```bash
cd examples/telegram-bridge
BRIDGE_ENV_FILE=/Users/you/fireline/deploy/telegram/bridge.env pnpm start
```

If the env file already lives in the current worktree at
`deploy/telegram/bridge.env`, `BRIDGE_ENV_FILE` is optional.

If `8787` is already occupied locally, override it for the smoke run:

```bash
BRIDGE_PORT=8788 BRIDGE_ENV_FILE=/Users/you/fireline/deploy/telegram/bridge.env pnpm start
```

If `TELEGRAM_CHAT_ID` is unset, send the bot one DM first so Telegram
`getUpdates` exposes a chat target for the startup "bridge online" ping.

## Health

```bash
curl -sf http://127.0.0.1:8787/healthz
```

`/healthz` returns `200` only when:

- Telegram `getMe` succeeds
- `FIRELINE_URL/healthz` is healthy
- `FIRELINE_STATE_STREAM_URL/healthz` is healthy
