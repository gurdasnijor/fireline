# Research: agentOS

This note captures what Fireline should borrow from rivet's agentOS and what it
should not.

## What agentOS gets right for our purposes

### Runtime/bootstrap owns process creation

In agentOS, `AgentOs.create()` and the runtime layer own sidecar spawn, VM
creation, and process bootstrap.

Their `AcpClient` is not a discovery client. It attaches to an already-owned
managed process.

That is a useful model for Fireline's TypeScript split:

- `client.host` owns runtime creation and discovery
- `client.acp` speaks ACP over a provided endpoint or transport

### Transport swap is an explicit seam

Their local stdio path and in-process transport split is a good testing and
integration pattern.

Fireline should preserve that same idea for:

- hosted WebSocket ACP
- local stdio attach
- in-memory test transports

### Local sidecar ergonomics are strong

agentOS makes local bring-up feel easy because runtime creation hides the Rust
sidecar details.

Fireline should borrow that ergonomics at the runtime layer, not by hiding
bootstrap inside the ACP client.

## What Fireline should not copy

### The in-process OS/kernel model

agentOS is building an in-process operating-system abstraction. Fireline is not.

Fireline should stay focused on:

- ACP conductors
- durable trace
- peer mediation
- runtime hosting boundaries

### Collapsing the control plane into the substrate

agentOS is more vertically integrated. Fireline should keep the Flamecast split
clear.

## Bottom line

The best thing to borrow from agentOS is the bootstrap/attach ergonomics and the
transport seam, not the execution model.
