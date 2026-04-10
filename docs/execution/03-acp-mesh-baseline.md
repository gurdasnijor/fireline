# 03: ACP Mesh Baseline

## Objective

Prove that one Fireline runtime can discover another and prompt it over ACP
without any REST side channel.

This slice stays on the SDK's session path:

- `session/new` is intercepted in `fireline-peer`
- the peer MCP server is injected with `with_mcp_server(...)`
- the proxy session is handed back to the SDK with
  `on_proxy_session_start(...)`
- `prompt_peer` uses a normal ACP client session against the remote `/acp`
  endpoint

## What this slice proves

- `fireline-peer` exposes `list_peers` and `prompt_peer` as MCP tools.
- Fireline runtimes register and unregister themselves in a shared local peer
  directory.
- `prompt_peer` reaches the remote runtime over ACP/WebSocket.
- The remote runtime records the cross-runtime prompt in its durable state
  stream.

## What remains deferred

- durable lineage projection from ACP `_meta` into state rows
- richer peer descriptors beyond the local file-backed directory
- reconnect / `session/load` across peer calls
- multi-hop / N-node causal stitching assertions

## Validation

- `tests/mesh_baseline.rs`
  - starts two Fireline runtimes with the SDK `Testy` agent
  - verifies tool discovery
  - verifies `list_peers`
  - verifies `prompt_peer`
  - verifies the remote runtime emits a `prompt_turn` state row
