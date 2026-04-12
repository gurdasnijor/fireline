# Session Migration

> The keynote demo: one session starts on a laptop, moves to another Fireline host mid-conversation, and keeps going because the durable stream never moved.

## Why this is uniquely Fireline

Most agent platforms bind conversation state to one process, one VM, or one proprietary session endpoint. Fireline splits the system into three planes:

- control plane: provision with `Sandbox.provision()`
- session plane: talk ACP to whatever sandbox is live now
- observation plane: durable stream is the source of truth

That means a sandbox can die, move, or be reprovisioned on another server while the conversation survives in the stream. The magic is not "better reconnect logic". The magic is **the session lived in the stream, not in the sandbox**.

## What this example shows

1. Provision on `http://127.0.0.1:4440`
2. Open an ACP session and send two turns
3. Provision the same harness on `https://remote:4440` with the **same** `stateStream`
4. Call ACP `session/load` on the remote sandbox
5. Continue the same conversation and inspect one durable state view that spans both hosts

## Prerequisites

Build the deterministic resumable test agent:

```bash
cargo build -q -p fireline --bin fireline --bin fireline-testy-load
```

Run two Fireline servers that point at the same durable-streams service:

```bash
fireline --durable-streams-url=http://127.0.0.1:7474 --host 127.0.0.1 --port 4440
fireline --durable-streams-url=http://127.0.0.1:7474 --host 127.0.0.1 --port 5440
```

Install deps for the example and run it:

```bash
cd examples/session-migration
pnpm install
FIRELINE_LOCAL_URL=http://127.0.0.1:4440 \
FIRELINE_REMOTE_URL=http://127.0.0.1:5440 \
pnpm start
```

## Why people say “holy cow”

- the second host does not need a session transfer API
- the first host does not hand off in-memory objects
- the browser or dashboard never changes read APIs
- one `@fireline/state` subscription sees the same session before and after the move

That combination only works because Fireline treats durable streams as the truth and ACP proxy chains as replaceable execution plumbing.
