# Live Monitoring

When people ask for "agent observability," they usually mean three things at once: which agents are active, which ones are stuck waiting for approval, and what the most recent work actually looks like. Most systems answer that by adding another service, another polling loop, and another custom API just for dashboards.

This demo shows the Fireline version of the story. The dashboard does not poll a control plane. It subscribes to the durable stream and asks normal live database questions: sessions, turns, approvals, tool calls. The UI updates because the stream is the system of record, not because the app wrote a pile of bespoke WebSocket code.

## What Happens

1. `createFirelineDB({ stateStreamUrl })` materializes the deployment state locally.
2. `useLiveQuery(...)` keeps sessions, turns, approvals, and tool calls live.
3. `useAcpClient(...)` is only there for the selected-session approval button.

## The Code

```tsx
const sessions = useLiveQuery((q) => q.from({ s: db.sessions }), [db])
const turns = useLiveQuery((q) => q.from({ t: db.promptTurns }), [db])
const approvals = useLiveQuery((q) => q.from({ p: db.permissions }), [db])
const toolCalls = useLiveQuery((q) => q.from({ c: db.chunks }), [db])
```

That is the product message. The monitoring surface is a live query over the durable stream, not a second-class reporting API.

## Run It

```bash
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/live-monitoring
pnpm install
VITE_FIRELINE_STATE_STREAM_URL=http://127.0.0.1:7474/streams/state/demo \
VITE_FIRELINE_ACP_URL=ws://127.0.0.1:4440/v1/acp/demo \
pnpm start
```

Point it at any Fireline state stream. If you also give it an ACP URL, the demo can approve the next pending permission from the UI.
