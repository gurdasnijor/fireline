# Managed-Agent Verification Layer

This directory adds a verification layer on top of Fireline's existing managed-agent contract tests. It does **not** replace the current integration suite. It gives the architecture three extra things:

1. a compact abstract state machine for the seven primitives,
2. a Rust-native model checker for the highest-value races and retry paths,
3. a small pure semantic kernel for the highest-risk protocol and conductor decisions,
4. a refinement matrix that maps the formal artifacts back to the executable tests already in `tests/`.

## What Is Modeled

The verification layer is intentionally narrow and architecture-first.

- **Session**
  - append-only log growth
  - replay-from-offset suffix behavior
  - durability across runtime death
  - idempotent append under retry keyed by producer commit tuple `(producer_id, epoch, seq)`
- **Host**
  - provisioned runtimes are reachable and reusable
  - cold wake preserves `runtime_key` and changes `runtime_id`
  - reprovision preserves `session/load` semantics at the abstract level
- **Orchestration**
  - live `resume(session_id)` is a no-op
  - cold resume preserves `runtime_key` and changes `runtime_id`
  - concurrent resumes converge on one effective runtime identity
- **Harness**
  - every visible harness effect lands in Session
  - approval-gate release requires a matching approval resolution
  - first matching approval decision is stable under duplicate or late resolutions
- **Sandbox**
  - provision / execute / stop is modeled as a distinct primitive from Host runtime lifecycle
  - tool execution stays isolated from direct Session mutation
  - only registered tools execute through the sandbox capability surface
  - stopping a sandbox does not mutate Host runtime identity
- **Resources**
  - requested `{source_ref, mount_path}` pairs map into mounted resources
  - fs-backend writes become durable session evidence
- **Tools**
  - agent-visible descriptors are schema-only
  - transport and credential references do not leak into the descriptor projection

## What Is Not Modeled

This layer deliberately does **not** try to prove the whole runtime.

- ACP wire details
- durable-streams internals beyond append/replay/dedupe shape
- Docker/local provider implementation details
- shell internals
- full resource mounting behavior inside a launched shell process
- whole-program correctness of the Rust runtime

Two known gaps remain explicit:

- **Shell-visible mounts inside the launched runtime** are still a pending end-to-end contract, not a formal proof target here.
- **Crash-surviving pause/resume with a blocked prompt mid-flight** remains a deeper Harness proof obligation than the currently passing guarantees.

## Layout

- `spec/managed_agents.tla`
  - abstract TLA+ architecture model of the seven primitives
- `spec/ManagedAgents.cfg`
  - small finite configuration for TLC
- `stateright/`
  - Rust-native protocol models for resume races, approval races, and append/dedupe
  - session dedupe is modeled with explicit producer tuple semantics, not a synthetic scope key
- `../crates/fireline-semantics/`
  - pure semantic kernels shared by the verifier
  - includes Session / Resume / Approval transition helpers
  - includes conductor algebra and property tests for component composition laws
- `docs/refinement-matrix.md`
  - invariant-by-invariant mapping from model coverage to executable tests

## How This Relates To The Existing Test Suite

The contract vocabulary comes from the current managed-agent tests:

- `tests/managed_agent_session.rs`
- `tests/managed_agent_harness.rs`
- `tests/managed_agent_orchestration.rs`
- `tests/managed_agent_sandbox.rs`
- `tests/managed_agent_resources.rs`
- `tests/managed_agent_tools.rs`
- `tests/managed_agent_primitives_suite.rs`

Those tests remain the source of truth for executable substrate behavior. The verification layer complements them:

- the TLA+ spec states the architecture at the primitive boundary
- the Stateright models explore race/retry interleavings that integration tests cover only by example
- the semantic kernel captures the small pure transition rules that are most likely to drift if they stay implicit in handler code
- the conductor algebra tests validate Fireline-specific composition laws that sit above the substrate primitives but below product code
- the refinement matrix makes it explicit which invariants are live-tested, model-checked, only specified, or still pending
- some architectural claims, like "resume is composition rather than a separate scheduler," remain design properties documented by model structure rather than standalone checked invariants

## Local Commands

Run the Rust-native model checker:

```sh
cargo test -p fireline-verification
```

Run the pure semantic kernel and conductor algebra tests:

```sh
cargo test -p fireline-semantics
```

Run the current managed-agent contract tests:

```sh
cargo test --test managed_agent_session \
  --test managed_agent_harness \
  --test managed_agent_orchestration \
  --test managed_agent_sandbox \
  --test managed_agent_resources \
  --test managed_agent_tools \
  --test managed_agent_primitives_suite
```

Run the TLA+ model manually with TLC:

```sh
java -cp /path/to/tla2tools.jar tlc2.TLC \
  verification/spec/managed_agents.tla \
  -config verification/spec/ManagedAgents.cfg
```

## CI Posture

The **CI-ready** part of this layer is the Rust model-checking harness:

```sh
cargo test -p fireline-verification
```

That command is deterministic, cheap, and separate from the heavier integration suite.

The semantic kernel is also CI-ready:

```sh
cargo test -p fireline-semantics
```

That gives us a second fast lane for:

- pure Session / Resume / Approval transitions
- conductor algebra laws such as composition identity, associativity, init-only tool registration, and single-fire resource provisioning

The TLA+ spec is committed as a first-class artifact and is TLC-ready, but TLC is not wired into automation here. That is intentional: the Rust model checker is the lightweight automated gate; the TLA+ spec remains the authoritative abstract architecture description and can be run manually or wired into CI later without changing the model.
