# Package API Ergonomics Gaps

## Context

This note is based on the current `examples/flamecast-client/` port attempt. The goal there was not to build a toy demo, but to take a real copied Flamecast UI and rewire it onto Fireline primitives.

The result is mechanically functional enough to compile, but the implementation is ugly for a clear reason: the current package surfaces stop at `provision()` and raw state rows. A real app immediately needs four more things:

1. connect ACP using the returned handle
2. create or attach to ACP sessions
3. open durable state from the same handle
4. query that durable state in session-centric terms

Those steps are where the current packages force custom glue.

## The Evidence

The current port had to add a non-trivial compatibility layer:

- `examples/flamecast-client/server.ts`: `1098` lines
- `examples/flamecast-client/ui/fireline-client.ts`: `384` lines
- `examples/flamecast-client/ui/hooks/use-session-state.ts`: `330` lines
- `examples/flamecast-client/ui/fireline-types.ts`: `187` lines
- `examples/flamecast-client/shared/acp-node.ts`: `71` lines
- `examples/flamecast-client/shared/resolve-approval.ts`: `27` lines

The worst call sites are:

- `examples/flamecast-client/server.ts:434-449` provisions a sandbox, manually opens ACP, then manually creates a session.
- `examples/flamecast-client/server.ts:490-498` manually appends approval resolution events.
- `examples/flamecast-client/server.ts:590-619` manually boots `@fireline/state`, subscribes to permissions, and drives auto-approval.
- `examples/flamecast-client/ui/provider.tsx:22-37` fetches custom config, manually constructs the DB, and manually manages preload/close lifecycle.
- `examples/flamecast-client/ui/fireline-client.ts:160-377` rebuilds a large app-local client facade because the package surface is too low-level for a copied UI.
- `examples/flamecast-client/ui/hooks/use-session-state.ts:20-129` manually joins prompt turns, chunks, permissions, ACP transport, and filesystem access into one hook.

## Main Conclusion

`compose(...).start()` is not the ugly part. The ugliness starts immediately after it succeeds.

The package problem is not "Fireline needs to ship Flamecast." The problem is that Fireline currently exposes:

- provisioning
- raw admin reads
- raw state collections

but not the next operational layer that every real app needs:

- ACP connection helpers
- session lifecycle helpers
- state bootstrap helpers
- state selectors / projections
- approval workflow helpers that are easy to find and obviously connected to middleware

## Gap 1: No First-Class ACP Connection Helper

### What exists today

- `SandboxHandle` is only raw endpoints: `packages/client/src/types.ts:249-257`
- `Sandbox.provision()` only returns that handle: `packages/client/src/sandbox.ts:56-69`

### What that forced

- a custom Node ACP bridge in `examples/flamecast-client/shared/acp-node.ts:29-63`
- manual ACP connection bootstrap in `examples/flamecast-client/server.ts:447-449`
- direct `use-acp` wiring in `examples/flamecast-client/ui/hooks/use-session-state.ts:30-38`

### Why this is a package gap

The handle already has the ACP endpoint. Fireline should not force every app to remember how to turn `handle.acp.url` plus optional headers into:

- a Node ACP SDK connection
- a browser `use-acp` config
- a consistent attach / close lifecycle

### Fix

Add a package-level ACP bridge:

- `connectAcp(handle)` for Node
- `acpConfig(handle, sessionId?)` for browser hooks
- `SandboxHandle.connect()` if the team wants it object-oriented

This does not need to hide the ACP SDK. It just needs to eliminate bespoke transport bootstrapping.

## Gap 2: No Session Primitive Above a Provisioned Sandbox

### What exists today

- `Sandbox.provision()` provisions infrastructure, not sessions: `packages/client/src/sandbox.ts:56-69`
- `SandboxAdmin` only supports `get`, `list`, `destroy`, `status`, `healthCheck`: `packages/client/src/admin.ts:25-69`, `97-165`

### What that forced

The example had to invent its own "session service" on top of raw ACP:

- create a sandbox
- open ACP
- call `newSession`
- hold the connection in memory
- invent REST routes for prompt, cancel, status, terminate

See `examples/flamecast-client/server.ts:430-488`.

### Why this is a package gap

The current package surface makes provisioning easy and session lifecycle manual. That is the wrong cut line for real apps. The first thing most apps want after provisioning is "start or attach to a conversation/session and send prompts."

### Fix

Add a thin session layer that sits above ACP, not above Flamecast:

- `startSession(handle, { cwd, mcpServers })`
- `attachSession(handle, sessionId)`
- returned object with `prompt()`, `cancel()`, `close()`, `sessionId`

This should stay Fireline-primitive-shaped. It does not need Flamecast concepts like agent templates or queueing.

## Gap 3: No Handle-to-State Bootstrap Helper

### What exists today

- `createFirelineDB()` only accepts a raw stream URL: `packages/state/src/collection.ts:39-63`
- there is no helper from `SandboxHandle` or `SandboxDescriptor` to a live DB

### What that forced

- a custom `/api/fireline-config` endpoint
- provider-level boot logic that fetches config, constructs the DB, calls `preload()`, and manages teardown manually

See `examples/flamecast-client/ui/provider.tsx:22-45`.

### Why this is a package gap

Fireline already knows that `handle.state` is the durable observation surface. Apps should not need to hand-stitch:

- state URL discovery
- preload lifecycle
- close lifecycle
- shared DB instance ownership

### Fix

Add one or both:

- `openFirelineState(handle | descriptor)`
- `createFirelineDBFromEndpoint(endpoint)`

And likely a React-friendly companion:

- `@fireline/state/react`
- `FirelineStateProvider`
- `useFirelineDb()`

The current low-level API is fine to keep, but it is not enough as the only public path.

## Gap 4: No Session-Centric Selectors on Top of Raw State Collections

### What exists today

`@fireline/state` exports raw collections such as:

- `promptTurns`
- `permissions`
- `chunks`

from `packages/state/src/collection.ts:21-35`.

### What that forced

The example had to manually reassemble session UI state:

- query turns
- query permissions
- query chunks
- filter chunks by turn ids
- sort everything
- synthesize logs
- derive pending approvals
- derive "is processing"

See `examples/flamecast-client/ui/hooks/use-session-state.ts:20-129`.

### Why this is a package gap

This is where the copied UI got noisy. The current package boundary is too low-level for any session UI. Every consumer will end up rewriting the same projections:

- transcript for session X
- pending approvals for session X
- active turn state for session X
- connection state for session X

### Fix

Keep raw collections, but add selectors or derived queries:

- `selectSessionTranscript(db, sessionId)`
- `selectPendingPermissions(db, sessionId)`
- `selectSessionStatus(db, sessionId)`
- `selectRuntimeSessions(db, runtimeId)`

If React helpers are acceptable, expose them from a dedicated subpath rather than bloating the core package.

## Gap 5: Approval Workflow API Is Fragmented and Poorly Signposted

### What exists today

- middleware exposes `approve({ scope: 'tool_calls' })`: `packages/client/src/middleware.ts:24-41`
- but the current translation is still a prompt-wide fallback: `packages/client/src/sandbox.ts:179-197`
- approval resolution exists in a separate subpath: `packages/client/src/events.ts:1-26`
- root exports do not surface it: `packages/client/src/index.ts:1-20`

### What that forced

The example reimplemented approval resolution locally in `examples/flamecast-client/shared/resolve-approval.ts:1-25` and wired it manually in `examples/flamecast-client/server.ts:490-498`, `602-619`.

### Why this is a package gap

There are two problems here:

1. discoverability: the event helper exists, but it is easy to miss
2. semantic drift: `approve({ scope: 'tool_calls' })` reads as if tool-call approval exists now, but the client maps it to a prompt-level gate fallback

That combination encourages accidental reimplementation.

### Fix

- make approval helpers part of the obvious public path
- rename or document the current fallback honestly
- add a single "approval workflow" module that pairs:
  - middleware declaration
  - pending approval observation
  - approval resolution append helpers

## Gap 6: No First-Class File / Resource Browsing Surface for Running Sandboxes

### What exists today

The public package surface gives provisioning and resource declaration, but not a runtime file inspection API. `resources.ts` defines mounts, not browsing: `packages/client/src/resources.ts:1-99`.

### What that forced

The example had to add custom backend endpoints for:

- file preview
- filesystem snapshot
- git branches
- git worktrees
- slash commands

See `examples/flamecast-client/ui/fireline-client.ts:197-223`, `259-376` and the matching server routes in `examples/flamecast-client/server.ts:220-287`.

### Why this is a package gap

Once a sandbox is running, apps immediately want to inspect the mounted workspace. Today there is no canonical Fireline-side way to say:

- read file
- walk directory
- inspect git state

So the example fell back to host-side filesystem shims.

### Fix

Pick one explicit path and make it public:

- ACP fs helpers, if ACP is the canonical substrate
- provider-backed sandbox filesystem endpoints, if that is the intended operator surface
- state-projected filesystem metadata, if the team wants durable observation first

What matters is that the package contract answer this question directly.

## Gap 7: Admin Surface Is Too Raw for Operator Apps

### What exists today

`SandboxAdmin` is intentionally small: `get`, `list`, `destroy`, `status`, `healthCheck` from `packages/client/src/admin.ts:25-69`.

### What that forced

The example server had to keep its own registries for:

- runtimes
- sessions
- templates
- settings
- queue state

and then expose custom REST routes like `/api/runtimes` and `/api/agents`.

### Why this is partly a package gap

Some of that translation is expected because Flamecast has its own product model. But the current admin surface is missing a few primitives that operator apps almost certainly need:

- server-side label filters
- `stop()`
- `waitUntilReady()`
- streaming observation of descriptor changes

Without those, `SandboxAdmin` is hard to build on.

### Fix

Keep admin separate from the primitive surface, but make it more operationally complete.

## Gap 8: Package Boundary Is Awkward to Consume From Real Examples

### What exists today

The example had to path-map and relative-import package sources directly:

- `examples/flamecast-client/tsconfig.json:16-20`
- `examples/flamecast-client/server.ts:7-11`
- `examples/flamecast-client/ui/fireline-client.ts:1-2`

### Why this is a package gap

This is not purely an API issue. It is a package-consumption issue. A real example should not need:

- `../../packages/client/src/...`
- `../../packages/state/src/...`
- TS path aliases to raw source files

That makes examples feel like internal test harnesses rather than normal consumers.

### Fix

Make examples consume the built package contract, not source files:

- workspace dependency wiring for examples
- stable subpath exports
- no example-local path aliasing to raw package internals

## Gap 9: Public Contract Budget Is Spent on Lower-Value Exports While Higher-Value Helpers Are Missing

### What exists today

The root client export still prioritizes topology combinators:

- `packages/client/src/index.ts:1-2`

while the truly missing ergonomic pieces are not there:

- ACP connect helper
- session helper
- state bootstrap helper
- approval workflow helper

### Why this matters

The issue is not that `fanout`, `peer`, or `pipe` are bad. The issue is priority. The root API is optimized for composition authoring, while the demo pain is in composition execution and observation.

### Fix

Rebalance the public surface toward the lifecycle most apps actually live in:

1. compose
2. provision
3. connect
4. observe
5. operate

## What Is Not a Package Gap

Not every ugly line in the Flamecast port should move into Fireline packages.

These are still app-level concerns:

- Flamecast agent templates
- Flamecast message queue UX
- Flamecast-specific REST route names
- Flamecast-specific session and runtime view models

The package fix is not "ship Flamecast." The package fix is "stop forcing apps to rebuild ACP/session/state glue."

## Recommended Cut Order

If the goal is to make the next example dramatically cleaner, the highest-leverage additions are:

1. `connectAcp(handle)` and browser ACP config helpers
2. `startSession(handle, opts)` / `attachSession(handle, sessionId)`
3. `openFirelineState(handle | descriptor)`
4. session selectors over `@fireline/state`
5. a coherent approval workflow surface

If those five land, most of the current shim layer disappears. The Flamecast demo would still need some app-specific adaptation, but it would stop looking like a second control plane.
