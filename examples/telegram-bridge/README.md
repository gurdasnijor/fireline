# Telegram Bridge

`telegram()` middleware is the primary product path on `main`. This example is
for the adjacent problem: you already have a Fireline host running, but the
operator who needs to approve tool calls is away from the terminal.

This bridge turns Telegram into a lightweight approval pager.

What it does:

- boots Chat SDK with the Telegram adapter in polling mode
- watches `fireline.db({ stateStreamUrl })` for pending approvals
- posts inline `Approve` / `Deny` cards into a Telegram DM or group
- resolves button clicks back into Fireline with `appendApprovalResolved(...)`
- answers `status`, `pending`, `approve`, and `deny` commands from chat
- exposes `GET /healthz` so you can tell whether Telegram, the host, and the
  state stream are all reachable

What it is **not**:

- not the signature demo path
- not a replacement for `telegram()` middleware inside a Fireline spec
- not a secrets relay

Use it when you want a small sidecar process that pages a human in Telegram
while the real agent runtime keeps running somewhere else.

## Run

1. Start the Fireline host you want to observe.

   The host prints a durable stream URL in its startup output. Copy that exact
   `state:` value. This bridge watches the stream directly; `http://127.0.0.1:7474`
   by itself is not enough.

2. Point the bridge env at that running host and stream.

```bash
cp deploy/telegram/bridge.env.example deploy/telegram/bridge.env
$EDITOR deploy/telegram/bridge.env
```

Minimum variables:

- `TELEGRAM_BOT_TOKEN`
- `FIRELINE_STATE_STREAM_URL` set to the exact stream URL from `fireline run`

Helpful optional variables:

- `TELEGRAM_CHAT_ID` if you want approval cards to land in one fixed chat
- `TELEGRAM_ALLOWED_USER_IDS` to restrict who can operate the bridge
- `FIRELINE_URL` if the host health probe is not `http://127.0.0.1:4440`

3. Start the bridge.

```bash
cd examples/telegram-bridge
pnpm install --ignore-workspace --lockfile=false
BRIDGE_ENV_FILE=/Users/you/fireline/deploy/telegram/bridge.env pnpm start
```

If `deploy/telegram/bridge.env` already exists in the same checkout,
`BRIDGE_ENV_FILE` is optional.

## Try It

1. DM the bot `status`.
2. Trigger a gated tool call in the host you are watching.
3. Wait for the approval card to arrive in Telegram.
4. Tap `Approve` or `Deny`.

The original Fireline run resumes from the same durable session. If buttons are
not convenient, you can also type:

```text
approve <session-id> <request-id>
deny <session-id> <request-id>
pending
```

## Commands

- `status` probes Telegram, `FIRELINE_URL/healthz`, and the durable stream
- `pending` lists every approval the bridge can still resolve
- `approve <session-id> <request-id>` resolves a specific pending approval
- `deny <session-id> <request-id>` denies a specific pending approval
- `help` prints the command summary

## Health

```bash
curl -sf http://127.0.0.1:8787/healthz
```

`/healthz` returns `200` only when:

- Telegram `getMe` succeeds
- `FIRELINE_URL/healthz` is healthy
- the state-stream server is healthy
- the approval watcher is connected

## Why this example exists

The product surface is still `telegram(...)` middleware. This example shows the
lower-level building blocks behind that story:

- Chat SDK + the Telegram adapter for operator delivery
- `fireline.db(...)` for durable approval observation
- `appendApprovalResolved(...)` for out-of-band approval decisions

If you want the shipped Telegram agent path instead of the sidecar reference,
use [docs/guide/telegram.md](../../docs/guide/telegram.md) and
[examples/telegram-demo](../telegram-demo/README.md).
