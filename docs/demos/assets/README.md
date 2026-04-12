# Demo assets — pi-acp → OpenClaw

Locked TypeScript specs used by the demo. Kept small and frozen so the
operator script and rehearsal captures stay reproducible.

## Files

- `agent.ts` — the north-star agent. Matches `docs/demos/pi-acp-to-openclaw.md` §3
  verbatim. 15 lines of spec: sandbox + middleware (trace / approve /
  budget / secretsProxy / peer(['reviewer'])) + `agent(['pi-acp'])`.
- `reviewer.ts` — the peer agent for Step 4 of the demo. Read-only mount,
  reciprocal `peer(['agent'])`, same trace + approve middleware.

## Invariants (do not change without updating the operator script)

1. Both files stay under 20 lines each.
2. Both use `agent(['pi-acp'])` — the public ACP registry id. Compose
   integration falls back per `docs/proposals/acp-registry-execution.md §Phase 3`
   if the registry is unreachable at runtime.
3. The middleware ordering in `agent.ts` is `[trace, approve, budget,
   secretsProxy, peer]` — operator narration references this order when
   explaining Step 1.
4. `ANTHROPIC_API_KEY` is sourced from `env:ANTHROPIC_API_KEY` (operator
   pre-flight P4 verifies).
5. `GITHUB_TOKEN` is scoped to `api.github.com` only — narration point in
   Step 1.

## Running

Per the operator script:

```bash
npx fireline docs/demos/assets/agent.ts
```

Reviewer launches in its own host in Step 4:

```bash
npx fireline docs/demos/assets/reviewer.ts
```

## Why frozen here and not under `examples/`

`examples/` is the runnable-demonstration surface for users; it evolves
with the API. The demo assets must be stable across rehearsals — a
change to `examples/` should never silently change what the operator
shows on stage.
