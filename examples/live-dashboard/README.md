# Live Dashboard

> A reactive observation demo: one dashboard subscribes to Fireline's durable stream and shows sessions, turns, approvals, and child-session lineage for an entire deployment with no polling loop and no custom dashboard API.

## Why this is uniquely Fireline

Most agent platforms need a bespoke observer service, a polling control plane, or an app-specific WebSocket fanout before you can build an operations UI. Fireline does not. The durable stream is already the source of truth:

- `createFirelineDB({ stateStreamUrl })` materializes the deployment state locally
- `useLiveQuery` reacts to new rows as they land
- `childSessionEdges` turns multi-agent topology into a live graph
- `use-acp` handles the optional ACP permission UX for a selected session without turning Fireline into an ACP wrapper

The magic is simple: **the stream is the dashboard API**.

## What this example shows

- a live count of sessions, turns, pending approvals, and child-session edges
- a selected-session panel driven by `useAcpClient({ wsUrl, autoConnect: true })`
- approval buttons powered by ACP while every other panel comes from `@fireline/state`

## Run it

```bash
pnpm --dir .. install --ignore-workspace --lockfile=false
cd examples/live-dashboard
pnpm install
VITE_FIRELINE_STATE_STREAM_URL=http://127.0.0.1:7474/streams/state/demo \
VITE_FIRELINE_ACP_URL=ws://127.0.0.1:4440/v1/acp/demo \
pnpm start
```

Point `VITE_FIRELINE_STATE_STREAM_URL` at any shared Fireline state stream and `VITE_FIRELINE_ACP_URL` at any live `handle.acp.url`. The stream drives the dashboard; ACP is only there for selected-session controls like approval.
