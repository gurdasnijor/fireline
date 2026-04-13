# Temporal Agent

This example is a tiny ACP agent written against raw stdio JSON-RPC. It does not use an LLM and it does not depend on anything beyond the Node standard library. The point is narrower: prove that an ACP agent can discover Fireline's temporal platform extensions and call them directly.

## What It Does

The agent implements the ACP handshake and three prompt branches:

1. `wait 5s` -> calls `session/wait`
2. `schedule hello in 10s` -> calls `session/schedule`
3. `wait for event` -> calls `session/wait_for`
4. any other prompt -> echoes the text and ends the turn

It uses `initialize.params.serverCapabilities.platform` as the discovery surface for those temporal primitives. This is intentionally a platform extension, not a claim that `session/wait`, `session/schedule`, or `session/wait_for` are part of base ACP today.

## Discovery Contract

The example accepts this capability shape:

```json
{
  "serverCapabilities": {
    "platform": {
      "temporal": {
        "methods": [
          "session/wait",
          "session/schedule",
          "session/wait_for"
        ]
      }
    }
  }
}
```

It also tolerates boolean forms such as:

```json
{
  "serverCapabilities": {
    "platform": {
      "temporal": {
        "wait": true,
        "schedule": true,
        "waitFor": true
      }
    }
  }
}
```

If a method is not advertised, the agent falls back to an ordinary `session/update` echo instead of trying to call the extension blindly.

## Extension Requests Emitted By This Example

This example emits the following platform-specific requests:

- `session/wait` with `{ sessionId, ms: 5000, durationMs: 5000 }`
- `session/schedule` with `{ sessionId, delayMs: 10000, ms: 10000, prompt: [{ type: "text", text: "hello" }] }`
- `session/wait_for` with `{ sessionId, filter: { kind: "event", name: "demo.temporal" } }`

Those payloads are the concrete contract of this demo example. They are meant to exercise the Flamecast -> Restate -> session-host -> ACP-agent -> temporal-primitive path end to end, not to define a new ACP standard.

## Run It

From the repo root:

```bash
node examples/temporal-agent/index.js
```

Or from the example directory:

```bash
cd examples/temporal-agent
node index.js
```

The process speaks newline-delimited JSON-RPC over stdio, so it is meant to be launched by a session host, harness, or test driver rather than typed into interactively.

## Smoke Test

```bash
cd examples/temporal-agent
node --test test/smoke.test.mjs
```

The smoke test simulates the ACP client side of:

- `initialize`
- `session/new`
- `session/prompt`
- the three temporal extension responses

## Register As An Agent Template

If you want to use this in the Flamecast example server's template registry, the minimal template body is:

```json
{
  "name": "temporal-agent",
  "spawn": {
    "command": "node",
    "args": ["examples/temporal-agent/index.js"]
  },
  "runtime": {
    "provider": "local"
  }
}
```

That matches the current `AgentTemplate` shape in `examples/flamecast-client/ui/fireline-types.ts`.

## Why This Exists

This is an end-to-end substrate check, not a product demo. If this agent can:

- receive an ACP prompt,
- discover temporal primitives from the host,
- call them over the same JSON-RPC channel,
- and resume cleanly when the host responds,

then the temporal chain is alive without involving an LLM or a heavyweight adapter.
