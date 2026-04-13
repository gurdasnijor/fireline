# Approval Gate Correctness Review

Date: 2026-04-12

## Scope

This review covers the current implementation in `crates/fireline-harness/src/approval.rs` as shipped today, without starting the broader DurableSubscriber generalization proposed in `docs/proposals/durable-subscriber.md`.

As of the tool-call interception follow-up, `approve({ scope: 'tool_calls' })` no longer lowers to a prompt-wide fallback. Tool-scoped approval now intercepts ACP `session/request_permission` and keys durable replay on canonical `(session_id, tool_call_id)`. Prompt-wide approval remains available only through explicit prompt-scoped config such as `approve({ scope: 'prompts' })`.

## What Was Proved

1. Crash and resume round-trip is now exercised end to end.
   The previously ignored harness test `harness_durable_suspend_resume_round_trip` now runs in `tests/managed_agent_harness.rs` and passes. It proves that a pending approval survives runtime death via durable-log replay and that a resumed runtime can continue the session without minting a fresh permission request.

2. Permission-request ids are now restart-stable.
   `approval.rs` no longer generates a fresh UUID on every emit. It derives a deterministic id from `(session_id, policy_id, prompt identity)`, where prompt identity prefers the Fireline trace id when present and otherwise falls back to stable prompt serialization. This closes the orphaned-permission bug on crash between gate entry and append durability.

3. Timeout behavior is observable and bounded.
   `harness_approval_gate_timeout_errors_cleanly` proves that a blocked prompt returns a structured gate error when `timeoutMs` elapses and that the durable stream contains only the emitted `permission_request`, not a phantom resolution. This required a real fix in `approval.rs`: gate failures now go through `responder.respond_with_error(...)` instead of bubbling out as transport-level teardown.

4. Concurrent approvals are isolated by `(session_id, request_id)`.
   The direct component test `concurrent_waiters_are_isolated_by_session_and_request_id` in `approval.rs` starts two concurrent waiters against a real durable-streams server, resolves only session A, and proves that session B remains blocked until its own `approval_resolved` arrives.

5. Rebuild races with live resolution are handled correctly.
   `harness_approval_resolution_during_rebuild_reuses_pending_request` proves that a fresh runtime can rebuild pending approval state from the log while an `approval_resolved` event is appended concurrently, and still release the resumed prompt without duplicating the permission request id.

## What Was Not Proved

1. End-to-end multi-session isolation through the full harness path.
   I could not get a same-runtime harness test to prove "session A resolves while session B is still suspended" in a stable way. The runtime path does not currently provide a dependable proof that two ACP prompt requests will both be resident in the approval gate concurrently. The actual isolation property is still proved, but it is proved one layer lower in `approval.rs`.

2. Full control-plane based approval proofs after the Host -> Sandbox rename wave.
   The control-plane test surface is still mid-migration across `host_key` vs `sandbox_id`, `HostDescriptor` vs `SandboxHandle`, and `/v1/runtimes` vs `/v1/sandboxes`. That is exactly the identifier drift being addressed in `docs/proposals/acp-canonical-identifiers.md`. I intentionally stopped before chasing that broader rename fallout.

3. A first-class prompt-turn-derived request id.
   The audit asked for `(session_id, prompt_turn_id, policy_id)`. The gate does not have a first-class `prompt_turn_id` at prompt-intercept time today, so the implementation uses Fireline trace id when available and a stable prompt fingerprint otherwise. That is sufficient for current crash/idempotency correctness, but it is not the same thing as a canonical turn id.

## Why The Session-Isolation Proof Lives In `approval.rs`

The harness layer is reliable for prompt timeout and crash/rebuild proofs, but it is not a trustworthy place to prove two same-runtime suspended prompts are simultaneously resident in the gate. The ACP/runtime path does not currently give a stable, testable guarantee that two prompt requests will both reach the suspend point concurrently inside one runtime process. The isolation property that matters for correctness is therefore proved directly at the gate's durable-stream wait layer.

## Remaining Edge Cases

1. Prompt-scoped approval is still session-scoped after the first allow.
   Once a session is marked approved, later matching prompts in that same session bypass the gate. This is the implementation the harness tests depend on today. It is safe for the current design, but it is weaker than prompt-turn-scoped approval and should not be silently carried into DurableSubscriber generalization.

2. Deterministic ids use prompt identity, not a first-class `prompt_turn_id`.
   The gate runs before the state projector materializes a prompt-turn row, so `prompt_turn_id` is not directly available on `PromptRequest`. The implementation uses the Fireline trace id when present, which becomes the downstream turn identity, and otherwise falls back to a stable prompt fingerprint.

3. `rebuild_from_log` is full-stream replay.
   Correctness is fine, but latency grows with stream size because the gate scans from offset 0 on `session/load`. DurableSubscriber should keep the correctness contract while tightening replay cost.

## Conclusion

At the component level, the approval gate is now strong enough to serve as the semantic reference for DurableSubscriber: suspend/resume durability, restart-stable ids, timeout behavior, and concurrent resolution isolation are all covered. At the end-to-end API and harness boundary, though, the broader identifier surface is still too unstable to generalize confidently. My assessment is:

- The internal durable-subscriber mechanics are safe to extract now.
- The external contract and cross-layer test story should wait for `docs/proposals/acp-canonical-identifiers.md` to land, because today the missing canonical id surface is exactly what forces prompt-identity fallback and blocks cleaner end-to-end proofs.
