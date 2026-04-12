# SecretsInjectionComponent

## TL;DR

`SecretsInjectionComponent` is the harness-layer component that resolves
`CredentialRef`s and injects the resulting plaintext into outbound tool
execution paths without ever exposing those values to the agent-visible
`ToolDescriptor` surface. It is the concrete follow-on to the sketch in
[`deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md)
Section 5 and is designed to live in `fireline-harness` alongside
[`budget.rs`](../../crates/fireline-harness/src/budget.rs).

The design deliberately splits into two paths:

1. The long-term path: intercept outbound MCP `tools/call` requests and
   apply `ToolArg` / `McpServerHeader` injections at call time.
2. The pragmatic demo-era path: support session-scope `EnvVar`
   injection at sandbox spawn time, which does not require a new `sacp`
   hook.

The component remains a harness concern either way: the rules,
resolver policy, audit events, and revocation semantics live at the
harness layer even when the actual `EnvVar` write happens inside the
sandbox provider.

## 1. Goals

- Keep the Anthropic-shaped `ToolDescriptor` schema-only. Credentials
  must never cross the agent boundary.
- Reuse the existing `CredentialRef` model from
  [`crates/fireline-tools/src/lib.rs`](../../crates/fireline-tools/src/lib.rs)
  and [`packages/client/src/core/tool.ts`](../../packages/client/src/core/tool.ts).
- Support both local development and remote production with the same
  component API and two resolver implementations.
- Make credential resolution auditable and revocation-driven without
  logging plaintext values.
- Ship a useful first slice before `sacp` exposes a typed outbound
  tool-call interception hook.

## 2. Rust Type Surface

The public type surface should be small and explicit. The component
holds immutable rules, a credential resolver, a small in-memory cache
for `Session` / `Once` scope, and an optional durable-streams producer
for audit events.

```rust
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use durable_streams::{Client as DurableStreamsClient, Producer};
use fireline_tools::CredentialRef;
use lru::LruCache;
use sacp::schema::SessionId;
use tokio::sync::RwLock;
use zeroize::Zeroizing;

#[derive(Clone)]
pub struct SecretsInjectionComponent {
    /// Pluggable secret lookup path. Local dev and durable-streams
    /// production both satisfy this trait.
    resolver: Arc<dyn CredentialResolver>,
    /// Immutable injection policy evaluated in declaration order.
    rules: Arc<[InjectionRule]>,
    /// Optional producer used to emit `credential_injected` and
    /// `credential_revoked` audit events. When absent, injection still
    /// works but no audit envelopes are written.
    audit_producer: Option<Producer>,
    /// Session-scoped and once-scoped resolved values. The cache stores
    /// `Arc<SecretValue>` to avoid cloning plaintext strings.
    session_cache: Arc<RwLock<HashMap<SessionCacheKey, Arc<SecretValue>>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InjectionRule {
    pub target: InjectionTarget,
    pub credential_ref: CredentialRef,
    pub scope: InjectionScope,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum InjectionTarget {
    /// Set an environment variable in the sandbox before the target
    /// worker starts.
    EnvVar(String),
    /// Add an auth header to outbound requests for the named MCP server.
    McpServerHeader { server: String, header: String },
    /// Write the resolved value into a specific tool argument path.
    ToolArg { tool: String, arg_path: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InjectionScope {
    /// Resolve once at session start and pin for the session lifetime.
    Session,
    /// Resolve on every tool invocation.
    PerCall,
    /// Resolve the first time the rule is used and then reuse until
    /// revoked.
    Once,
}

#[async_trait]
pub trait CredentialResolver: Send + Sync {
    async fn resolve(
        &self,
        credential_ref: &CredentialRef,
        session_id: &SessionId,
    ) -> Result<SecretValue, CredentialResolverError>;
}

/// Wrapper around plaintext secret material. This type must not derive
/// `Serialize` or a plaintext `Debug`; it exists only as an in-memory
/// carrier.
pub struct SecretValue(Zeroizing<String>);

impl SecretValue {
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

#[derive(Debug)]
pub enum CredentialResolverError {
    NotFound { credential_ref_name: String },
    Forbidden { credential_ref_name: String, reason: Option<String> },
    Expired {
        credential_ref_name: String,
        expired_at_ms: Option<u64>,
    },
    Transport {
        store: &'static str,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct SessionCacheKey {
    session_id: SessionId,
    rule_index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ResolverCacheKey {
    scope: String,
    credential_ref_name: String,
    envelope_seq: u64,
}

struct ResolverCacheEntry {
    value: Arc<SecretValue>,
    cached_at: Instant,
}
```

### Notes on the type surface

- `CredentialRef` stays the portable handle. The component never
  accepts raw secret values in configuration.
- `SecretValue` intentionally avoids `Clone`. Any internal sharing
  should happen via `Arc<SecretValue>` so the implementation does not
  multiply plaintext copies in memory.
- `CredentialResolverError` variants are named for policy and
  operational outcomes rather than backing stores. The caller can map
  them to `sacp::Error` without leaking backend-specific details.
- `InjectionRule` is intentionally minimal. If future slices need
  conditional matching, that belongs in a wrapper config, not in the
  credential carrier itself.

## 3. Resolution Model and Rule Evaluation

### 3.1 Credential reference naming

Audit events and cache keys need a stable, opaque identifier for a
`CredentialRef` without exposing the value. The proposal uses the
following canonical names:

- `CredentialRef::Env { var }` -> `env:<var>`
- `CredentialRef::Secret { key }` -> `secret:<key>`
- `CredentialRef::OauthToken { provider, account }` ->
  `oauth:<provider>:<account-or-default>`

This canonical name is what appears in audit envelopes and resolver
cache keys.

### 3.2 Rule application semantics

Rules are evaluated in declaration order, but conflicting targets
should be rejected at configuration load time. A single component
instance should not accept two rules that both write to:

- the same `EnvVar(NAME)`
- the same `McpServerHeader { server, header }`
- the same `ToolArg { tool, arg_path }`

That keeps the runtime semantics deterministic and the audit trail easy
to reason about.

### 3.3 Scope semantics

- `Session`: resolve once per session before the first use and hold in
  `session_cache` until the session ends or a revocation event removes
  it.
- `PerCall`: resolve every time the rule is applied. No cache other
  than any transport-local retry cache the resolver itself needs.
- `Once`: resolve the first time the rule is applied and cache until a
  revocation event invalidates it. This differs from `Session` only for
  rules that may never be used in a session.

The demo-era fallback only supports `InjectionTarget::EnvVar` with
`Session` scope end to end. `Once` can be implemented for `EnvVar` only
if the sandbox provider applies the same cached value on each worker
launch. `PerCall`, `ToolArg`, and `McpServerHeader` need the outbound
tool-call intercept described below.

## 4. `ConnectTo<Conductor>` Integration

### 4.1 Reference pattern

`BudgetComponent` in
[`crates/fireline-harness/src/budget.rs`](../../crates/fireline-harness/src/budget.rs)
shows the expected harness shape:

- a small stateful component object
- `impl ConnectTo<sacp::Conductor>`
- a `sacp::Proxy::builder()` chain
- typed interception on an ACP request flowing through the conductor

That pattern works cleanly for prompt budgeting because the prompt path
already has a typed request surface (`PromptRequest`) and the component
can attach at `on_receive_request_from(Client, ...)` in
`budget.rs:152-189`.

### 4.2 Desired intercept point for secrets

Secrets need a different interception point: the agent-originated MCP
`tools/call` request after tool selection but before transport.

Semantically, the component wants:

1. the current `session_id`
2. the tool name
3. the JSON argument payload
4. the selected MCP server / transport target
5. a mutable request object so headers and JSON arguments can be
   rewritten before the call leaves the conductor

The exact typed request does not exist in the current repository.
`budget.rs:16-20` and `approval.rs:18-24` both call out the same gap:
tool calls currently travel as MCP-over-ACP and do not present a clean,
typed proxy hook.

### 4.3 Proposed `ConnectTo<Conductor>` sketch

Once `sacp` grows the hook, the implementation should mirror the budget
proxy closely:

```rust
impl ConnectTo<sacp::Conductor> for SecretsInjectionComponent {
    async fn connect_to(self, client: impl ConnectTo<sacp::Proxy>) -> Result<(), sacp::Error> {
        let this = self.clone();
        sacp::Proxy
            .builder()
            .name("fireline-secrets")
            .on_receive_request_from(
                sacp::Agent,
                {
                    let this = this.clone();
                    async move |request: CallToolRequest, responder, cx| {
                        let session_id = request.session_id().to_string();
                        let tool_name = request.tool_name().to_string();
                        let transport = request.transport_target().clone();

                        let rewritten = this
                            .inject_into_tool_call(&session_id, &tool_name, &transport, request)
                            .await?;

                        cx.send_request_to(sacp::Client, rewritten)
                            .forward_response_to(responder)
                    }
                },
                sacp::on_receive_request!(),
            )
            .connect_to(client)
            .await
    }
}
```

`CallToolRequest` here is intentionally schematic. The important part
is the required seam: a typed, mutable outbound tool invocation hook on
the agent-to-MCP leg.

### 4.4 What happens before that hook exists

Do not block the whole feature on upstream `sacp`.

PR 4 should ship a narrower path first:

1. `SecretsInjectionComponent` owns rules, resolver selection, cache,
   audit emission, and revocation handling.
2. The component exposes a helper such as
   `resolve_session_env(session_id) -> HashMap<String, Arc<SecretValue>>`
   for `InjectionTarget::EnvVar` + `InjectionScope::Session`.
3. The sandbox provider calls that helper at worker spawn time and
   merges the resulting env map into the worker process environment.

This path does not require any proxy-level hook because the injection
happens before the sandbox worker process exists.

### 4.5 Limits of the fallback path

The sandbox-spawn fallback is intentionally scoped:

- It supports `EnvVar` only.
- It gives correct demo-era behavior for per-session secrets, which is
  enough for API keys and similar credentials.
- It does not solve per-call `ToolArg` mutation or per-server header
  injection.
- On long-lived workers, a session-scoped env var remains pinned for
  the worker lifetime. Revocation invalidates the resolver cache
  immediately, but a worker that already inherited the env will not be
  scrubbed until restart. That is acceptable for the first shipped
  slice and should be documented as a limitation in PR 4.

## 5. CredentialResolver Implementations

### 5.1 `LocalCredentialResolver`

This is the developer-mode resolver. It must read fresh from disk on
every `resolve()` and intentionally keep no cache.

```rust
pub struct LocalCredentialResolver {
    pub toml_path: PathBuf,
    pub env_fallback: bool,
    pub gh_fallback: bool,
    pub aws_fallback: bool,
}
```

Resolution order:

1. `~/.config/fireline/secrets.toml`
2. environment variables when `env_fallback == true`
3. `~/.config/gh/hosts.yml` when `gh_fallback == true`
4. `~/.aws/credentials` `[default]` when `aws_fallback == true`

Proposed local file layout:

```toml
[secrets]
openai_api_key = "..."
anthropic_api_key = "..."

[oauth.github]
default = "gho_xxx"
work = "gho_yyy"
```

Lookup rules:

- `CredentialRef::Env { var }`: read `std::env::var(var)` directly.
- `CredentialRef::Secret { key }`: read `[secrets].<key>` from TOML
  first; if absent and `env_fallback` is enabled, try an environment
  variable derived from `key` in upper snake case.
- `CredentialRef::OauthToken { provider, account }`: read
  `[oauth.<provider>].<account-or-default>` from TOML first; for
  `provider == "github"`, fall back to `hosts.yml` if enabled.
- AWS fallback is specifically for secrets such as
  `aws_access_key_id`, `aws_secret_access_key`, and
  `aws_session_token` when they are not present in the Fireline TOML.

Rationale for no cache: local development is the rotation path. If a
user edits `~/.config/fireline/secrets.toml`, the next resolve should
see the new value without a daemon restart or cache flush.

### 5.2 `DurableStreamsCredentialResolver`

This is the remote / production resolver.

```rust
pub struct DurableStreamsCredentialResolver {
    pub client: DurableStreamsClient,
    pub scope: String,
    pub private_key: Arc<age::x25519::Identity>,
    pub cache: tokio::sync::Mutex<LruCache<ResolverCacheKey, ResolverCacheEntry>>,
    pub cache_ttl: Duration,
}
```

Secrets live on a stream named `secrets:<scope>`. The scope should be a
stable project identifier by default, matching the recommendation in
`deployment-and-remote-handoff.md` Section 9 Question 2.

Each durable-streams envelope should be encrypted at rest and should
carry only opaque metadata plus ciphertext:

```json
{
  "kind": "secret_envelope",
  "scope": "project:demo-app",
  "credential_ref_name": "secret:gh_token",
  "envelope_seq": 42,
  "created_at_ms": 1775846349405,
  "ciphertext": "age-encrypted-payload"
}
```

Recommended crypto:

- crate: `age`
- recipient type: `x25519`
- rationale: small API surface, audited ecosystem, and trivial deploy
  UX with `age-keygen` plus public-key distribution

Resolution flow:

1. Read the latest `secret_envelope` for the target
   `credential_ref_name` from `secrets:<scope>`.
2. Use `(scope, credential_ref_name, envelope_seq)` as the correctness
   key for the cache.
3. If a fresh cache entry exists and `cached_at + cache_ttl` is still
   live, reuse it.
4. Otherwise decrypt the envelope on demand, wrap it immediately in
   `Zeroizing<String>`, place it behind `Arc<SecretValue>`, and store it
   in the LRU.

The cache TTL should start at 5 minutes. That is a policy guess, not a
correctness guarantee, and needs validation against the expected
revocation SLA.

### Revocation and rotation

Rotation is append-only: a new `secret_envelope` with a higher
`envelope_seq` supersedes older entries. Revocation is also append-only
and should invalidate cache entries immediately:

```json
{
  "kind": "credential_revoked",
  "scope": "project:demo-app",
  "credential_ref_name": "secret:gh_token",
  "revoked_at_ms": 1775846355123
}
```

When the resolver or component sees a `credential_revoked` event for
`(scope, credential_ref_name)`, it must evict all matching cache keys
regardless of sequence number.

## 6. Enforced Invariants and Audit Envelopes

The deployment proposal lists four invariants. This section ties each
one to an implementation boundary.

### 6.1 `ToolDescriptorNoCredentialLeak`

This invariant is already structurally enforced by the current tools
surface.

- `fireline_tools::ToolDescriptor` contains only
  `{ name, description, input_schema }`.
- `CapabilityRef` is the launch-time attachment that carries
  `transport_ref` and optional `credential_ref`.
- `emit_tool_descriptor()` projects only the descriptor half to the
  durable state stream.

`SecretsInjectionComponent` does not read or mutate `ToolDescriptor`s.
It only resolves credentials and rewrites outbound tool-call payloads
after tool selection. There is therefore no descriptor leak surface for
the component to close. This invariant is enforced structurally by the
type split in `crates/fireline-tools/src/lib.rs`.

### 6.2 Durable stream never logs raw credentials

The component should emit exactly two envelope families:

```json
{
  "kind": "credential_injected",
  "session_id": "sess_123",
  "credential_ref_name": "secret:gh_token",
  "target_kind": "env_var",
  "resolved_at_ms": 1775846349405
}
```

```json
{
  "kind": "credential_revoked",
  "scope": "project:demo-app",
  "credential_ref_name": "secret:gh_token",
  "revoked_at_ms": 1775846355123
}
```

Neither envelope shape contains a plaintext value.

Implementation rules:

- `SecretValue` must not implement `Serialize`.
- `SecretValue` debug output must always redact.
- `CredentialResolverError` must name only the reference, never the
  resolved value.
- Any `tracing` or audit call inside the component must log only
  `session_id`, `scope`, `credential_ref_name`, `target_kind`, and
  timestamps.

### 6.3 Injection is auditable via `credential_injected`

Every successful injection writes a `credential_injected` envelope to
the session stream. The event is keyed for audit only; it is not a
replay input because replay reconstructs runtime effects from the
captured post-injection tool outputs, not from raw secret material.

Proposed Rust event type:

```rust
use sacp::schema::SessionId;

#[derive(serde::Serialize)]
struct CredentialInjectedEvent {
    kind: &'static str,
    session_id: SessionId,
    credential_ref_name: String,
    target_kind: &'static str,
    resolved_at_ms: u64,
}
```

No value field is present.

### 6.4 Revocation is a stream event

Revocation is append-only and distributed through the stream, not
through a central side channel.

Proposed Rust event type:

```rust
#[derive(serde::Serialize)]
struct CredentialRevokedEvent {
    kind: &'static str,
    scope: String,
    credential_ref_name: String,
    revoked_at_ms: u64,
}
```

Cache invalidation rule:

- on receipt of `credential_revoked`, drop all cache entries for the
  matching `(scope, credential_ref_name)`
- on the next eligible injection, `resolve()` must fetch a newer
  envelope or fail with `CredentialResolverError::Forbidden` /
  `NotFound`

## 7. Proposed TLA Invariant Additions

These are future Level 3 additions to
`verification/spec/managed_agents.tla`. They are proposals only; no
TLA changes are part of this document.

- `CredentialRefResolvedBeforeToolCall`
  Plain English: any tool call that requires an injection target must
  have a preceding `credential_injected` event in the same session log.
  Extends: `HarnessAppendOrderStable` and `SessionAppendOnly`.
- `ResolvedCredentialNeverInLog`
  Plain English: no session-log envelope payload contains the plaintext
  form of any resolved credential. In the abstract model this is
  structural because values are not modeled, but the invariant should
  still exist to make the design intent explicit.
  Extends: `ToolDescriptorNoCredentialLeak`.
- `RevokedCredentialNotReused`
  Plain English: after a `credential_revoked` event for a given
  `credential_ref_name`, no later tool call in the same session may use
  that credential unless a newer secret envelope supersedes it.
  Extends: `HarnessAppendOrderStable` and the future
  `CredentialRefResolvedBeforeToolCall`.

## 8. Migration and Rollout Plan

1. PR 1: add `crates/fireline-harness/src/secrets.rs` with the type
   surface only. No resolver implementations yet. Unit tests cover
   config validation, redacted debug behavior, and event serialization.
2. PR 2: add `LocalCredentialResolver` plus unit tests. Feature flag:
   `local-secrets`.
3. PR 3: add `DurableStreamsCredentialResolver` plus an integration
   test against a real durable-streams-server and an encrypted
   round-trip. Feature flag: `durable-streams-secrets`. This is the PR
   that pulls in `age`.
4. PR 4: add the first `ConnectTo<Conductor>` integration and wire the
   component into the harness topology. Ship session-scope `EnvVar`
   injection end to end via the sandbox-spawn fallback path.
5. PR 5: emit `credential_injected` / `credential_revoked` events on the
   session stream and update the refinement matrix / docs to account for
   the new audit envelopes.
6. PR 6: add `fireline sync-secrets` so local secrets can be encrypted
   and uploaded to the remote durable-streams secrets stream.

## 9. Dependencies and Blocked-By

- `SecretsInjectionComponent` belongs in `fireline-harness`, which is
  stable only after the current crate restructure finishes.
- `DurableStreamsCredentialResolver` is not blocked by Fireline code,
  but it does require an available durable-streams-server instance for
  the integration test.
- Full `ToolArg` / `McpServerHeader` support depends on a `sacp`-level
  outbound tool-call hook that does not exist in the current codebase.
  PR 4 should proceed with sandbox-spawn `EnvVar` injection first and
  leave the proxy-hook path for a later follow-up.

## 10. Open Questions

- Key distribution UX: how does the user obtain the deploy public key
  locally so `fireline sync-secrets` can encrypt uploads? The current
  recommendation is a `fireline init` flow that generates a keypair,
  prints the public key, and emits a shell snippet for installing the
  private half on the host.
- Per-project vs per-user scoping: the deployment proposal recommends
  per-project `secrets:<scope>`. That is still the right default, but
  it should be confirmed before the remote resolver lands.
- Cache TTL: 5 minutes is a placeholder. The implementation needs an
  explicit revocation propagation target before finalizing this.
- OAuth resolver: the deployment proposal's Section 5.3 options imply a
  future `OAuthCredentialResolver`. The trait above is compatible with
  that, but provider-specific refresh semantics and browser handoff are
  out of scope for the first slice.
- Env-var mapping for `CredentialRef::Secret`: upper-snake fallback is
  pragmatic but should be confirmed before baking it into user-facing
  docs.
- Long-lived worker semantics: if a sandbox provider keeps a worker
  alive for the session lifetime, revocation cannot scrub an inherited
  env var without restarting that worker. The first implementation
  should document this explicitly rather than implying stronger
  semantics than it has.

## 11. Why the `BudgetComponent` Pattern Only Partially Applies

The budget component is still the right structural template, but the
match is incomplete in three specific places:

- `crates/fireline-harness/src/budget.rs:16-20`
  The budget component explicitly says tool-call dispatch does not
  present a clean proxy hook today. Secrets has the same dependency.
- `crates/fireline-harness/src/budget.rs:152-189`
  The actual budget `ConnectTo<Conductor>` implementation works because
  it intercepts a typed `PromptRequest` from the client-facing side.
  Secrets needs an agent-to-MCP hook, not a prompt hook.
- `crates/fireline-harness/src/approval.rs:18-24`
  Approval already documents the exact same limitation for tool-call
  gating. That is additional evidence that the outbound call seam needs
  to be added in `sacp`, not worked around with prompt-level policy for
  secrets.

## 12. References

- [`docs/proposals/deployment-and-remote-handoff.md`](./deployment-and-remote-handoff.md)
- [`docs/proposals/runtime-host-split.md`](./runtime-host-split.md)
- [`crates/fireline-tools/src/lib.rs`](../../crates/fireline-tools/src/lib.rs)
- [`packages/client/src/core/tool.ts`](../../packages/client/src/core/tool.ts)
- [`crates/fireline-harness/src/budget.rs`](../../crates/fireline-harness/src/budget.rs)
- [`crates/fireline-harness/src/approval.rs`](../../crates/fireline-harness/src/approval.rs)
