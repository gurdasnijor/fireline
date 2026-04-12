# Cross-host discovery

This demo proves two Fireline control planes can provision agents on separate
hosts, publish into the same `hosts:tenant-default` stream, and discover each
other through `peer_mcp`.

## Run

```bash
cargo build --bin fireline --bin fireline-streams --bin fireline-testy
cd examples/cross-host-discovery
pnpm install
pnpm run dev
```

The `dev` script starts:

- `fireline-streams` on `http://127.0.0.1:7474/v1/stream`
- a control plane on `http://127.0.0.1:4440`
- a control plane on `http://127.0.0.1:5440`
- this example client

The client provisions `agent-a` on `:4440` and `agent-b` on `:5440`, connects
to `agent-b`, calls `fireline-peer.list_peers`, then calls
`fireline-peer.prompt_peer` to reach `agent-a` over ACP.
