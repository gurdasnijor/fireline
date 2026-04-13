# Cross-Host Discovery

Most multi-agent demos quietly cheat. They put every agent on one host, wire a
service registry behind the curtain, or hard-code the callee endpoint into the
caller. That is not the product problem teams hit in production. The real
question is simpler and harsher:

**If two Fireline hosts boot on different machines, can one agent discover the
other through shared infrastructure that already exists?**

This example shows the answer on current `main`: yes, as long as both hosts
publish into the same durable-streams deployment. The stream is the discovery
plane.

## What This Example Proves

- One runtime can boot on `http://127.0.0.1:4440` while another boots on
  `http://127.0.0.1:5440`.
- Both hosts publish presence into the same durable-streams deployment.
- The caller uses `fireline-peer.list_peers` to discover the remote runtime.
- The caller uses `fireline-peer.prompt_peer` to hand work to the remote
  runtime over ACP.
- The callee stream records the delegated child-session work on the remote
  host, so you can prove the handoff happened from durable state instead of
  trusting transient stdout.

What it does **not** prove:

- discovery across different durable-streams deployments
- DNS-based service lookup
- a separate control-plane registry service

If you point the two hosts at different stream backends, the example fails with
`peer '<name>' not found`. That is the important contract to understand.

## Why The Shape Looks Like This

The example starts two different control planes but keeps two different ideas
separate:

- **Shared discovery plane**
  Both control planes publish into one durable-streams deployment. That shared
  deployment stream is how `peer()` discovers runtimes.
- **Per-runtime session streams**
  Each runtime still writes its own session history into its own state stream.
  Discovery is shared; transcript state is per runtime.

That split is the user-facing design point. Fireline does not need a second
registry service just to let agents find each other.

## Run It

```bash
cargo build --bin fireline --bin fireline-streams --bin fireline-testy
cd examples/cross-host-discovery
pnpm install
pnpm run dev
```

The `dev` script starts:

- `fireline-streams` on `http://127.0.0.1:7474/v1/stream`
- a Fireline control plane on `http://127.0.0.1:4440`
- a Fireline control plane on `http://127.0.0.1:5440`
- the example client

The client provisions:

- `inventory-west-<run-id>` on host A
- `dispatcher-east-<run-id>` on host B

Then it does two things from the caller session on host B:

1. call `fireline-peer.list_peers`
2. call `fireline-peer.prompt_peer` to reach `inventory-west`

Expected output shape:

```json
{
  "question": "Can agents on different hosts find each other through the stream instead of a service registry?",
  "topology": {
    "sharedDiscoveryPlane": "Both control planes publish into the same durable-streams deployment."
  },
  "peersVisible": ["inventory-west-<run-id>", "dispatcher-east-<run-id>"],
  "remoteHandoff": {
    "target": "inventory-west-<run-id>",
    "responseText": "order 4815 is reserved in the west warehouse and can ship today"
  }
}
```

The output also includes:

- the caller and callee state-stream URLs
- the caller session id
- the callee child-session id
- a callee transcript excerpt plus completed prompt-turn summaries from the
  remote host

## Environment Overrides

- `FIRELINE_REGION_A_URL`
  Override host A. Default: `http://127.0.0.1:4440`
- `FIRELINE_REGION_B_URL`
  Override host B. Default: `http://127.0.0.1:5440`
- `AGENT_COMMAND`
  Override the ACP agent command. Default:
  `../../target/debug/fireline-testy`
- `REMOTE_MESSAGE`
  Override the delegated prompt content
- `CALLER_AGENT_NAME`
  Rename the caller runtime. Default:
  `dispatcher-east-<per-run suffix>`
- `CALLEE_AGENT_NAME`
  Rename the remote runtime. Default:
  `inventory-west-<per-run suffix>`

## Why The Default Agent Is `fireline-testy`

This example is about the discovery plane, not model quality. The default local
ACP stub keeps the demo deterministic and credential-free, so you can verify
the topology without mixing in provider latency or model variance.

If you want a model-backed version, set `AGENT_COMMAND` to another ACP-speaking
agent. The discovery story does not change.

## Read The Output Correctly

When the demo succeeds, the important facts are:

- `peersVisible` contains both runtimes even though they live on different
  hosts
- `remoteHandoff.responseText` matches the delegated request
- `stateEvidence.calleeCompletedTurns` is non-empty, proving the remote host
  materialized its own child session

If you already have other demo runtimes sharing the same local durable-streams
deployment, the raw `list_peers` tool result may include them too. This example
filters the headline summary down to the two runtimes it provisioned so the
product moment stays readable. It also appends a per-run suffix to the default
runtime names so repeated local runs do not collide with older `prompt_peer`
targets on the same shared stream service.

That is the product claim behind Fireline's peer fleet story:

**the stream is already the registry.**
