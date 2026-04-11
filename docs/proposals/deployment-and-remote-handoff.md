# Deployment and Remote Handoff

## TL;DR

**Two deployable artifacts, one hub, one binary.** Local development
and remote production run the *exact same* `fireline` binary — the
only difference is config. The durable-streams service doubles as the
transport for session logs, state, trace events, file mounts, and
encrypted secret envelopes. That collapses the local→remote handoff
from "rebuild everything for the cloud" into "point your local
fireline at a different URL."

The demo narrative this enables is the single strongest story on the
roadmap: **a session that started on a user's laptop can migrate
mid-conversation to a remote node without losing a single token of
state, because the durable-streams service never moved.** Wake
semantics (`WakeOnReadyIsNoop`, `WakeOnStoppedChangesRuntimeId`,
`WakeOnStoppedPreservesSessionBinding`) keep working across the
topology change for free — those invariants are stated in
`verification/spec/managed_agents.tla` over the session log, not over
the physical Host identity.

## 1. Deployment topology

Three nodes, all externally addressable over the network:

```
┌─────────────────────────────────────────────────────────────┐
│  Durable Streams Service  (well-known URL)                   │
│  - Deployed as: durable-streams-server docker image          │
│    https://thesampaton.github.io/durable-streams-rust-server │
│  - Single source of truth for:                               │
│      • session logs                                          │
│      • session state envelopes                               │
│      • trace events                                          │
│      • file/document transfer (NEW — blob streams)           │
│      • encrypted secret envelopes (NEW — secrets streams)    │
│  - Reachable by: every fireline Host + every sandbox VM      │
│  - Survives: Host death, sandbox death, network partition    │
└─────────────────────────────────────────────────────────────┘
                          ↑
        ┌─────────────────┼─────────────────┐
        │                 │                 │
┌───────┴──────┐  ┌───────┴──────┐  ┌──────┴───────┐
│  fireline    │  │  fireline    │  │  fireline    │
│  Host A      │  │  Host B      │  │  Host (local)│
│  (cloud)     │  │  (cloud)     │  │  (laptop)    │
│              │  │              │  │              │
│  - bin: same │  │  - bin: same │  │  - bin: same │
│    fireline  │  │    fireline  │  │    fireline  │
│  - Deployed: │  │  - Deployed: │  │  - bare      │
│    OCI image │  │    OCI image │  │    metal     │
│  - Sandbox:  │  │  - Sandbox:  │  │  - Sandbox:  │
│    micro-    │  │    micro-    │  │    local     │
│    sandbox   │  │    sandbox   │  │    subproc   │
│    daemon    │  │    daemon    │  │    or micro- │
│              │  │              │  │    sandbox   │
└──────────────┘  └──────────────┘  └──────────────┘
        │                 │                 │
        ▼                 ▼                 ▼
   ┌─────────┐       ┌─────────┐       ┌─────────┐
   │ Sandbox │       │ Sandbox │       │ Sandbox │
   │ VM (OCI │       │ VM (OCI │       │ VM or   │
   │ image)  │       │ image)  │       │ local   │
   └─────────┘       └─────────┘       │ process │
                                       └─────────┘
```

### The two deployable artifacts

1. **`fireline` OCI image** — the Host binary, containerized. Same
   binary you run locally. Deploy via any container orchestrator
   (k8s, Nomad, fly.io, ECS, bare Docker).
2. **`durable-streams-server` OCI image** — the external, well-known
   durable log service. Deploy once per environment. It has an
   existing production deployment guide at
   https://thesampaton.github.io/durable-streams-rust-server/deployment/production.html

**`microsandbox`** is not an "artifact" in the deployment sense —
it's *host infrastructure*, installed on each fireline node the same
way Docker is installed. microsandbox then consumes standard OCI
images per
https://docs.microsandbox.dev/images/overview#oci-images, so the
user's "deploy my agent" story is just "push a Dockerfile to a
registry." No new packaging format, no build system, no lock-in.

### Key architectural consequence

The durable-streams service is the **only stateful** component in the
topology. fireline Hosts are stateless — any Host can resume any
session by reading from the shared stream. microsandbox VMs are
stateless — they boot from OCI images, do their work, write captured
effects back to the stream. This is what makes the topology
dynamic: **nodes can come and go without losing session state.**

## 2. Local and remote run the same binary

The fireline binary takes its deployment posture from config alone:

```bash
# Local dev (laptop, experimentation, tests)
fireline \
  --durable-streams-url=http://localhost:7474 \
  --sandbox-provider=local-subprocess

# Remote production (cloud, k8s pod, VM)
fireline \
  --durable-streams-url=https://streams.prod.internal \
  --sandbox-provider=microsandbox
```

Everything going through the `Host` primitive (create session, wake,
status, stop) is primitive-identical. The ACP surface (`fireline-harness`)
is identical. The runtime HTTP API is identical. The only thing that
differs is **which sandbox provider** the Host uses to spawn tool
execution environments, and **which durable-streams URL** it writes
to.

This has a powerful implication for the handoff: **pointing your
local fireline at a cloud durable-streams URL is already a partial
migration**. Your session logs are now durable against a remote
service, and any other Host in the world that can reach the same URL
can resume your session. The sandboxes are the last thing to
migrate — and that's a per-session choice, not a deployment choice.

## 3. The handoff: two real friction points

When a user moves from "I'm experimenting on my laptop" to "I want
this to run in the cloud," two things that trivially work locally
suddenly don't:

### 3.1 Local context and documents

Agents work against files on the user's machine:

- The git repo they're editing
- Reference documents they've copied into `~/projects/notes/`
- `.env` files, config files, data fixtures
- Screenshots, PDFs, CSVs the user is asking questions about

Remote fireline Hosts don't have any of that. A naive "sync my
filesystem to the cloud" is both too much (you don't want to upload
`/home/user` wholesale) and too little (sync happens once, diverges
after).

### 3.2 Local security and trust

Agents need credentials to be useful:

- Anthropic / OpenAI / other LLM API keys
- `gh auth login` session token
- AWS / GCP / Azure credentials
- Private registry pull tokens
- MCP server API keys
- Database connection strings

Remote fireline Hosts don't have any of those either. And even if
you copied them — you now have a secret sprawl problem: every cloud
Host has a copy of every credential, nothing rotates, audit is
lost.

## 4. Solving context and documents

### Use the Resources primitive with durable-streams as the transport

`ResourceRef` is already `{ source_ref, mount_path }` in the
client-primitives contract. Today `source_ref` is effectively a local
path. Extend it with three new variants:

```rust
enum ResourceSourceRef {
    LocalPath(PathBuf),                                     // existing
    DurableStreamBlob { stream: String, key: String },      // NEW
    OciImageLayer { image: String, path: String },          // NEW
    HttpUrl(Url),                                           // NEW
}
```

**The key insight**: the durable-streams server already has blob
storage. We don't need S3, we don't need a separate file service,
we don't need to build an artifact registry. The durable-streams
server becomes the **universal hub** — it's already reachable from
every fireline Host and every sandbox VM by definition.

### The sync flow

User declares intent with a normal `ResourceRef`:

```typescript
const handle = await host.createSession({
  agentCommand,
  resources: [
    { source_ref: { kind: 'local_path', path: '~/projects/foo' },
      mount_path: '/workspace/foo' },
  ],
  metadata: { ... },
})
```

When the Host is running **locally** and the resource source is
`local_path`, the mount works today via the existing `LocalPathMounter`.

When the user wants to migrate to remote, they run:

```bash
fireline sync-to-remote \
  --durable-streams-url=https://streams.prod.internal \
  ~/projects/foo \
  --resource-name=foo
```

This:

1. Reads the directory contents locally
2. Chunks them into a blob stream on the target durable-streams
   server (`resources:foo/*`)
3. Emits a manifest event with tree structure + content hashes
4. Returns a new `ResourceRef` that points at
   `DurableStreamBlob { stream: "resources:foo", key: "/" }`

The user embeds the returned ref in their `SessionSpec.resources`
instead of the local path, or — better — the fireline CLI rewrites it
automatically when it detects the resource is a local path and the
durable-streams URL is remote.

### Mount on the remote side

A remote fireline Host resuming the session reads the session log
(as it already does), sees the `DurableStreamBlob` reference, and
has a `DurableStreamMounter` (sibling of `LocalPathMounter` in
`fireline-resources`) that:

1. Reads the blob stream + manifest
2. Materializes the contents to a tmpfs under the sandbox's mount
   root
3. Captures outbound writes back into the same stream via
   `FsBackendComponent` — which we already have in
   `fireline-resources/src/fs_backend.rs`

**No new transport infrastructure.** The durable-streams server is
the universal file transport because it's already the universal
log transport, and the semantics are the same: append-only, durable,
replayable.

### Composition with the existing FsBackendComponent

Today `FsBackendComponent` captures sandbox-side writes as `fs_op`
envelopes on the session stream. After the extension above, the
same component:

- On **initialization**: reads the `DurableStreamBlob` manifest to
  materialize the initial mount
- On **writes**: captures them as `fs_op` envelopes (existing
  behavior)

So the read-side and write-side both go through the same stream.
Round-tripping a file from local → cloud → local works without
special-casing: the cloud Host wrote an `fs_op` to the stream, the
local Host's `FsBackendComponent` reads it back when syncing
locally.

## 5. Solving secrets and credentials

### A new SecretsInjectionComponent, sibling of BudgetComponent

Today `crates/fireline-harness/src/budget.rs` implements
`BudgetComponent` — a `ConnectTo<sacp::Conductor>` proxy component
that intercepts `PromptRequest` flowing through the ACP pipeline,
counts tokens, and can terminate a turn that exceeds a configured
ceiling. It runs at the harness layer, sees every request between
client and agent, and has a clean place to inject per-session logic.

**`SecretsInjectionComponent` uses the same pattern, different
payload.**

```rust
// crates/fireline-harness/src/secrets.rs  (sibling of budget.rs)

pub struct SecretsInjectionComponent {
    resolver: Arc<dyn CredentialResolver>,
    injections: Vec<InjectionRule>,
}

pub struct InjectionRule {
    pub target: InjectionTarget,
    pub credential_ref: CredentialRef,  // reuses CoreType from Tools primitive
    pub scope: InjectionScope,
}

pub enum InjectionTarget {
    /// Set an env var in the sandbox env before any tool spawns.
    EnvVar(String),
    /// Add a header to outbound MCP server requests for a named server.
    McpServerHeader { server: String, header: String },
    /// Inline a value into a tool call argument at a JSON-path.
    ToolArg { tool: String, arg_path: String },
}

pub enum InjectionScope {
    /// Resolve once at session start; pinned for the lifetime of the session.
    Session,
    /// Resolve at every tool invocation.
    PerCall,
    /// Resolve once, cache until revoked.
    Once,
}

#[async_trait]
pub trait CredentialResolver: Send + Sync {
    async fn resolve(
        &self,
        credential_ref: &CredentialRef,
        session_id: &str,
    ) -> Result<SecretValue>;
}

pub struct SecretValue(Zeroizing<String>);  // zeroizes on drop
```

### Invariants the component enforces

These map directly onto the TLA properties already checked in
`verification/spec/managed_agents.tla`:

1. **The agent never sees the raw credential.** It sees only the
   `ToolDescriptor` (schema only). This is the existing
   `ToolDescriptorNoCredentialLeak` invariant — the harness layer
   already enforces it on the descriptor projection path.
2. **The durable stream never logs raw credentials.** Only
   `CredentialRef`s (`CredentialRef::secret("gh_token")`) appear in
   the log; resolved values never get serialized. A new
   `credential_injected` event records the ref name + session + tool
   for audit, without the value.
3. **Injection is auditable and replay-safe.** Resolution happens at
   the harness layer; replay-from-offset reconstructs the session
   without needing the secret values because the sandbox outputs
   captured in the log are post-injection.
4. **Revocation is a stream event.** A `credential_revoked` envelope
   drops the cache on every fireline Host tailing the stream. No
   central revocation service needed.

### Two CredentialResolver implementations

**`LocalCredentialResolver`** — for dev:

```rust
pub struct LocalCredentialResolver {
    toml_path: PathBuf,             // ~/.config/fireline/secrets.toml
    env_fallback: bool,              // also check std::env::var
    gh_fallback: bool,               // parse ~/.config/gh/hosts.yml
    aws_fallback: bool,              // parse ~/.aws/credentials
}
```

Reads from familiar local sources. No encryption, no remote calls.
Good enough for laptop dev, not shipped to production.

**`DurableStreamsCredentialResolver`** — for production and the
remote half of the handoff:

```rust
pub struct DurableStreamsCredentialResolver {
    client: DurableStreamsClient,
    secrets_stream: String,          // "secrets:<scope>"
    private_key: Arc<PrivateKey>,    // deploy-time, mounted from
                                     // k8s secret / vault / env
}
```

Reads envelopes from a dedicated **secrets stream** on the
durable-streams server. Envelopes are **encrypted at rest** with a
deploy-time public key that the remote fireline Host has the private
half of. The durable-streams server never sees plaintext even if
compromised.

### The sync tool for secrets

```bash
fireline sync-secrets \
  --durable-streams-url=https://streams.prod.internal \
  --from-local \
  --encrypt-to=deploy-public-key.age \
  --scope=session-id-or-project
```

Steps:

1. Reads from local stores (`LocalCredentialResolver` paths)
2. Encrypts each secret with the provided public key
   (age / libsodium sealed box — library choice, not central to the
   design)
3. Appends each encrypted value as an envelope to the
   `secrets:<scope>` stream on the target durable-streams server,
   keyed by the `CredentialRef` name
4. Rotation is append-only: a new envelope for the same key with a
   newer timestamp supersedes; old envelopes stay for audit

**Critical invariants at the sync boundary**:

- Secrets **must** be encrypted before upload — never plaintext in
  transit
- The remote fireline Host has the private key mounted from its
  deploy-time secret store (k8s Secret, HashiCorp Vault, AWS Secrets
  Manager, etc.)
- The durable-streams server has no key material — compromise of the
  stream service is bounded to "attacker sees ciphertext"
- The local `fireline sync-secrets` tool **never** writes plaintext
  secrets to disk except the original local store the user already
  had

### The hard case: OAuth tokens from browser flows

The `gh auth login` case is harder because the credential was minted
via a local browser session. Three options, increasing rigor:

1. **File copy as a Resource** — treat `~/.config/gh/hosts.yml` as a
   Resource, let it flow through the file-sync pipeline in §4, and
   install it at the sandbox mount root. Simple; works today with
   no extra code. Downside: the token has full local scope and
   lives in a stream envelope for its lifetime.
2. **Token exchange at resolve time** — the `CredentialResolver`
   for an oauth provider knows how to run a short refresh-token
   flow. The user uploads only the refresh token (not the access
   token); the remote fireline Host mints fresh access tokens on
   demand and caches them in-memory. Access tokens are short-lived,
   so a stream leak is bounded. Requires the `CredentialResolver`
   to know the provider.
3. **Browser proxying** — the cloud fireline Host exposes
   `/v1/oauth/begin/:provider` and `/v1/oauth/callback/:provider`.
   The user runs `fireline remote-auth github --host-url=https://...`,
   which opens a local browser pointed at the cloud Host's oauth
   start route. The cloud Host completes the oauth handshake itself
   and stores the result in its own local secrets resolver. No
   local→remote token movement at all. Cleanest, but most work to
   ship.

**Recommended rollout**: ship option 1 for the demo era; design for
option 2 as the post-demo hardening path; option 3 only if
enterprise / compliance asks for it.

## 6. The demo narrative

This is the punchline, and it's why the handoff story is worth
front-loading into the demo.

### The "topology migration mid-session" beat

1. Open the browser harness. Launch an agent locally. Run a few
   prompts — the user sees them land in the session log via the
   state explorer panel.
2. Click a new "Migrate to Remote" button (or run a CLI equivalent).
   - The local fireline Host uploads resources and encrypted
     secrets to the cloud durable-streams server
   - A remote fireline Host is woken via `host.wake(handle)` pointing
     at the same `session_id` — `WakeOnStoppedChangesRuntimeId`
     applies: same `runtime_key`, new runtime identity
   - The local Host's wake returns `{ kind: 'noop' }` because from
     the session's point of view nothing new needs to happen —
     `WakeOnReadyIsNoop` applies
3. Send another prompt. It round-trips through the remote Host.
   **The session history is unbroken** — the state explorer shows
   turns from before and after the migration in a single continuous
   log.
4. Kill the remote Host (simulate a deploy, a crash, a region
   failover). Wake again. A different cloud node picks up the
   session, reads the log, mounts the resources from the blob
   stream, resolves credentials from the secrets stream, and keeps
   going. **Zero conversation state is lost.**

### Why this works

Every "magic" moment in that demo traces back to a property we
already proved in the TLA model:

| Demo moment | TLA invariant |
|---|---|
| Session history survives local→remote | `SessionDurableAcrossRuntimeDeath` |
| Wake on remote Host rehydrates correctly | `WakeOnStoppedPreservesSessionBinding` |
| Local Host's final wake is a no-op | `WakeOnReadyIsNoop` |
| Remote Host gets a new runtime id, same key | `WakeOnStoppedChangesRuntimeId` |
| Two concurrent wakes during migration don't double-provision | `ConcurrentWakeSingleWinner` |
| The state explorer's log never rewinds | `SessionAppendOnly` |
| Resources mounted on the remote sandbox match the local intent | `ResourceMountMappingCorrect` |
| Tool invocations on the remote side don't leak credentials through the descriptor | `ToolDescriptorNoCredentialLeak` |

**Every property is already checked in
`verification/spec/managed_agents.tla` today.** The deployment story
is not a new architecture — it's the physical manifestation of
invariants the formal spec has been encoding the whole time. That's
the best possible demo posture: "we designed this from the
primitives up; here's the distributed topology that falls out."

## 7. Implementation mapping

All of the above composes out of existing primitives. Nothing
requires a new crate or a new primitive — just concrete satisfiers,
new components, and two CLI subcommands.

| Deployment need | Primitive used | New work |
|---|---|---|
| Fireline binary in cloud | **Host** (`fireline-host`) | OCI Dockerfile, ~30 lines |
| Durable streams at well-known URL | external service | deployment config only, no Fireline code |
| Sandbox isolation from OCI images | **Sandbox** (`fireline-sandbox::MicrosandboxSandbox`) | already exists behind feature flag `microsandbox-provider`; needs image-pull wiring |
| Resources at the sandbox | **Resources** (`fireline-resources`) | extend `ResourceSourceRef` enum; new `DurableStreamMounter` |
| File sync transport | durable-streams blob streams | extend `FsBackendComponent` to read mount content from stream manifest |
| Secret injection | **Harness** Component | new `SecretsInjectionComponent` in `fireline-harness/src/secrets.rs`, ~300 lines |
| Credential resolution | **Tools** (`CredentialRef` already exists) | two `CredentialResolver` impls (local + durable-streams) |
| Local→cloud file sync | new `fireline sync-to-remote` CLI subcommand | ~200 lines, reuses durable-streams client |
| Local→cloud secret sync | new `fireline sync-secrets` CLI subcommand | ~150 lines + encryption layer |
| OAuth-minted credentials | post-demo hardening path (option 2 above) | new `OAuthCredentialResolver`, ~200 lines per provider |
| Browser-proxied oauth | post-demo | new `/v1/oauth/*` routes in `fireline-host`, ~300 lines |

## 8. Milestones and sequencing

Everything here is **post-restructure**. Do not start any of it until
`fireline-runtime` and `fireline-control-plane` have dissolved into
`fireline-host` and `cargo check --workspace` is green on the new
crate layout. See `docs/proposals/crate-restructure-manifest.md`
§"Execution status".

Then, in order of decreasing demo value:

### M1 — Minimal demo path (pre-demo polish, if time)

- Single `fireline` binary with `--durable-streams-url` required
- Browser harness talks to it as it does today
- **No migration UX yet**. The demo just shows "same binary, local
  mode," "same binary, pointed at a cloud durable-streams URL" —
  even as two separate runs, that's already a compelling story.

### M2 — Resource sync (first week post-demo)

- Extend `ResourceSourceRef` with `DurableStreamBlob` variant
- Add `DurableStreamMounter` sibling of `LocalPathMounter`
- Wire `FsBackendComponent` to read mount manifests from the stream
- Ship `fireline sync-to-remote` CLI subcommand
- Add an integration test: local Host uploads a directory, remote
  Host mounts it into a sandbox, captured writes round-trip back

### M3 — SecretsInjectionComponent + local resolver (second week)

- `crates/fireline-harness/src/secrets.rs` — component + trait +
  injection rules
- `LocalCredentialResolver` — reads from
  `~/.config/fireline/secrets.toml` + env + gh/aws fallbacks
- Wire the component through the harness topology
- Enforce the four invariants on the resolve path
- Integration test: tool call with a `CredentialRef::env("OPENAI_API_KEY")`
  resolves correctly, agent never sees the value, stream never logs
  it

### M4 — DurableStreamsCredentialResolver + sync tool (third week)

- `DurableStreamsCredentialResolver` reading encrypted envelopes
  from a secrets stream
- `fireline sync-secrets` CLI with age/libsodium encryption
- Key rotation via append-only envelopes
- Integration test: local sync → remote resolve → tool call works
  end-to-end

### M5 — Migration demo UX (fourth week)

- "Migrate to Remote" button in the browser harness
- Live demo: start a session locally, run prompts, migrate, continue
  prompting, kill the remote node, wake again, keep going
- Record it. **This is the keynote moment.**

### M6 — OAuth hardening (post-keynote, enterprise lane)

- Option 2: `OAuthCredentialResolver` with refresh-token exchange
- Option 3: `/v1/oauth/*` routes for browser-proxied flow

## 9. Open questions

1. **Key distribution**: the deploy-time public key for the secrets
   encryption — how does the user get it on first setup?
   `fireline init --generate-keys` that bootstraps a keypair and
   prints instructions for installing the private half on the
   remote Host?
2. **Multi-tenant secret scoping**: is `secrets:<scope>` per-session,
   per-project, or per-user? Recommendation: **per-project**, with
   the project ID as a stable identifier the user provides. Per-session
   is too granular (can't share credentials across sessions);
   per-user is too coarse (can't isolate projects).
3. **Blob stream garbage collection**: when resources are no longer
   referenced by any live session, do we leave them in the stream
   forever? Recommendation: **yes, for now**. Append-only storage
   is cheap, audit is more valuable than cleanup, and the durable-streams
   server has its own retention config for operators who need to
   reclaim space.
4. **Concurrent Host handoff**: two fireline Hosts both trying to
   wake the same session simultaneously during a migration.
   `ConcurrentWakeSingleWinner` says this converges, but the demo
   should show it explicitly — scripted test where two Hosts race,
   exactly one wins. Good to bank as a safety property.
5. **Sandbox-to-sandbox file transfer**: can sandbox A read a file
   written by sandbox B in the same session? Yes, via the
   `fs_op`-captured stream. Worth documenting as a deliberate
   capability, not an accident.
6. **Cost model**: writing every file to a durable-streams server
   costs storage and bandwidth. For large binary artifacts, is
   there a size cutoff where we fall back to an object store like
   S3 referenced by URL? Flag for M2 planning.

## 10. Non-goals (what this proposal explicitly doesn't cover)

- **Cross-region durable-streams replication.** Out of scope —
  the durable-streams service handles its own replication posture.
  Fireline Hosts don't care; they just point at a URL.
- **Control-plane federation.** Previously discussed and rejected
  in favor of "one binary, per-Host HTTP API." A cluster is N
  fireline binaries, each with its own listener. Routing is an
  infra concern (DNS, k8s service, service mesh).
- **Registry authentication for OCI image pulls.** Standard
  microsandbox / OCI flow; not a Fireline concern.
- **Secret scanning.** Preventing a user from accidentally
  committing secrets into the stream via a regex scan on envelope
  content. Potentially a post-demo hardening item; not in scope
  here.
- **End-user-facing "Migrate" UX polish.** The demo version is a
  single button that runs the sync commands and re-wakes against
  a different durable-streams URL. A proper enterprise UX (diff
  view, selective migration, preview of what'll move) is a post-
  keynote product lane.

---

**See also**:
- `docs/proposals/client-primitives.md` — the client-facing primitive surface this composes against
- `docs/proposals/runtime-host-split.md` §7 — the Host/Sandbox/Orchestrator taxonomy that makes this cleanly expressible
- `docs/proposals/crate-restructure-manifest.md` — the dependency graph this sits on top of
- `verification/spec/managed_agents.tla` — the formal invariants this relies on
- `crates/fireline-harness/src/budget.rs` — the reference pattern for `SecretsInjectionComponent`
