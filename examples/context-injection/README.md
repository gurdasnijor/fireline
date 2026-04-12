# Context Injection

> A proxy-chain demo for AGENTS.md, rules files, and hidden prompt transforms: Fireline injects workspace context into every ACP prompt before the agent sees it, and the transformation is observable in durable state.

## Why this is uniquely Fireline

Most agent platforms treat prompt hooks as app glue or proprietary server magic. Fireline makes them composable ACP proxy components:

- the context policy is declared in `middleware([...])`
- the user prompt stays visible as a durable `prompt_turn`
- the agent-visible prompt is changed by the proxy chain, not by the agent
- the effect is reusable across agents because it is middleware, not app code

This is the same architectural slot as `AGENTS.md`, repo rules, and custom hooks, but expressed as data in the harness spec.

## What this example shows

```ts
compose(
  sandbox({ resources: [localPath('/tmp/demo-workspace', '/workspace', true)] }),
  middleware([
    trace({ includeMethods: ['session/prompt'] }),
    contextInjection({ files: ['/workspace/RULES.md', '/workspace/CONTEXT.md'] }),
  ]),
  agent(['fireline-testy-prompt']),
).start({ serverUrl: 'http://127.0.0.1:4440' })
```

The demo uses `fireline-testy-prompt`, which echoes the exact prompt it saw. That makes the proxy-chain effect visible byte-for-byte:

- `promptTurns.text` = raw user prompt from the client
- `chunks.content` = injected prompt that reached the agent

## Run it

```bash
cargo build -q -p fireline --bin fireline --bin fireline-testy-prompt
cd examples/context-injection
pnpm install
pnpm start
```

The output prints both prompt views side by side. That is the point of the demo: **context lives in the proxy chain, not in the agent binary**.
