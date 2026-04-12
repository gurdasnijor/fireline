# App Code Ergonomics Audit

Date: 2026-04-11

Scope:
- `packages/browser-harness/src/app.tsx`
- Other non-test application code that consumes `@fireline/client`

Method:
- Read `packages/browser-harness/src/app.tsx` first, as requested.
- Searched `packages/` for non-test imports of `@fireline/client`, `@fireline/client/admin`, `@fireline/client/middleware`, and `@fireline/state`.
- Result: `packages/browser-harness/src/app.tsx` is the only in-tree application consumer. Other hits are tests or library code.

## Summary

The strongest ergonomics issue is not Fireline control-plane usage. `Sandbox`, `SandboxAdmin`, and `@fireline/state` are being used in a mostly clean way. The real friction is ACP setup: the browser harness spends well over half of its ACP connection code on generic WebSocket/SDK ceremony and required stub methods.

Findings by class:
- `API GAP`: 0 clear gaps in `@fireline/client` itself
- `APP BUG`: 1
- `LEGITIMATE`: 4
- `BOILERPLATE`: 2

Most important conclusion:
- The browser harness is **not** proving that `@fireline/client` needs to wrap ACP.
- It **is** proving that ACP browser bootstrapping needs a canonical snippet or an ACP-SDK-side default helper, because the same setup would otherwise be repeated across apps.

## Findings

| Classification | File:line | Finding | Recommendation |
|---|---|---|---|
| `BOILERPLATE` | `packages/browser-harness/src/app.tsx:269-297`; `1029-1096` | ACP connection setup is mostly generic ceremony. The app constructs a `WebSocket`, waits for open, adapts it into an ACP `Stream`, builds `ClientSideConnection`, and performs the initialize handshake. Rough count: about 69 lines for transport/stream/wait helpers, 14 lines for `ClientSideConnection` creation, and 9 lines for the initialize handshake. That is already more than 90 lines before any app-specific permission UI or event handling. | Do not add a Fireline ACP wrapper. Instead, provide a documented browser snippet/recipe, or have the ACP SDK expose a tiny default utility for “WebSocket to `ClientSideConnection`” plus a default unsupported-client handler. |
| `LEGITIMATE` | `packages/browser-harness/src/app.tsx:276-283`; `375-396`; `984-997` | The permission flow and session-update handling are app-specific. The harness needs to surface approval prompts in its own UI and append session updates to its own event log. That logic belongs in app code, not in `@fireline/client`. | Keep this in the app. If documented, show it as app-owned callback logic layered on top of a generic ACP connection recipe. |
| `BOILERPLATE` | `packages/browser-harness/src/app.tsx:999-1025` | `createClientHandler()` contains 27 lines of repetitive “not implemented” ACP client methods (`writeTextFile`, `createTerminal`, `extMethod`, etc.). This is unlikely to differ much across thin ACP browser clients, and every consumer would otherwise copy the same stubs. | This should be a documented ACP-SDK pattern or ACP-SDK helper, not a Fireline client abstraction. A `createUnsupportedClientHandler({ requestPermission, sessionUpdate })`-style helper in the ACP SDK would remove the repetition without violating the “no Fireline ACP wrapper” rule. |
| `LEGITIMATE` | `packages/browser-harness/src/app.tsx:456-460` | Passing `stateStream: STATE_STREAM_NAME` is not evidence that every consumer must pick a stream name. In the current harness, it is a deliberate choice to keep the browser harness on a stable, predictable stream for local inspection and deterministic smoke tests. The API already makes `stateStream` optional, so normal consumers can omit it and let the server generate one. | Keep the option as an escape hatch. Document the intended rule: ordinary apps omit `stateStream`; apps that need a stable named stream for long-lived inspection or deterministic tooling may set it explicitly. |
| `APP BUG` | `packages/browser-harness/src/app.tsx:164`; `456-460` | The harness UI text is more tightly coupled to the fixed stream name than it needs to be. The header claims the app is observing `/v1/stream/{STATE_STREAM_NAME}`, but the actual source of truth for observation is `handle.state.url`, which the app already receives and uses. If the explicit `stateStream` override goes away or changes, the header becomes misleading. | Derive the displayed stream path from the current `handle.state.url` once a sandbox is provisioned, or label the constant clearly as a harness-specific demo stream. |
| `LEGITIMATE` | `packages/browser-harness/src/app.tsx:62-84`; `799-965` | `@fireline/state` usage is clean. The app creates the DB from `handle.state.url`, calls `preload()`, closes it in cleanup, and uses `useLiveQuery` directly over collections. The ceremony is modest and React-appropriate; it does not feel like the app is fighting the state API. | No new abstraction is needed. At most, add a short React recipe in docs showing `createFirelineDB` + `preload` + `useLiveQuery`. |
| `LEGITIMATE` | `packages/browser-harness/src/app.tsx:183-190`; `426-487` | The split between `Sandbox` for provisioning and `SandboxAdmin` for status/destroy is reasonable. The app is not misusing the API here; it is using the narrow control-plane surface as designed. | Keep this pattern. The client API is minimal but still readable in the app. |
| `LEGITIMATE` | `packages/browser-harness/src/app.tsx:410-423`; `452-454` | The browser harness fetches `/api/agents` and `/api/resolve` from its own sidecar server to populate a launchable-agent picker. That is app/backend logic, not a sign that `@fireline/client` is missing a core control-plane primitive. | Keep it outside the core client surface. If product later wants agent selection/resolution in the SDK, it should live in a distinct layer, not be folded into `Sandbox`/`SandboxAdmin`. |

## Specific requested answers

### 1. Why does `createClientHandler` have to be implemented by the app?

Short answer:
- Partly because the app owns the permission UX and event log.
- Partly because the ACP SDK currently requires a full `Client` implementation, which forces repetitive stub methods.

Breakdown:
- `requestPermission` behavior is app-specific. The browser harness needs to show a dialog and resolve the user's choice. That belongs in app code.
- `sessionUpdate` handling is app-specific. The harness logs events into its own UI.
- The remaining unsupported ACP client methods are generic boilerplate. Those do **not** belong in every app.

Verdict:
- App-specific permission/update callbacks: `LEGITIMATE`
- Stubbed unsupported ACP methods: `BOILERPLATE`

Best fix:
- Not a Fireline wrapper.
- Prefer either:
  - an ACP SDK helper/default handler, or
  - a canonical documented snippet in Fireline docs that apps can copy verbatim.

### 2. Why is `stateStream` passed in the sandbox provision call?

Observed call:

```ts
sandboxClient.provision({
  ...compose(sandbox(), [trace()], agent(resolved.agentCommand)),
  name: 'browser-harness',
  stateStream: STATE_STREAM_NAME,
})
```

Assessment:
- This is **not** required by the client API. `stateStream` is optional.
- In this harness, the explicit stream name is serving a harness-specific need: deterministic local inspection and deterministic smoke tests.
- The real observation connection still comes from `handle.state.url`, which the app already uses.

Verdict:
- `LEGITIMATE` for this harness

Recommendation:
- Document that most apps should omit `stateStream`.
- Reserve explicit names for apps that intentionally want a stable durable stream identity.

### 3. ACP setup: how much is boilerplate vs app-specific?

Approximate line counts in `app.tsx`:
- WebSocket construction + stream adapter + open/close helpers: about 69 lines
- `ClientSideConnection` creation: about 14 lines
- `initialize()` handshake: about 9 lines
- Required unsupported-client stubs in `createClientHandler()`: about 27 lines
- App-specific permission/update wiring: about 11 lines
- Permission-resolution UI logic: about 22 lines

Interpretation:
- Well over 50% of the ACP setup is generic boilerplate.
- The genuinely app-specific part is mostly the permission UI and event logging.

Verdict:
- `BOILERPLATE`

Recommendation:
- Provide a well-documented browser ACP snippet.
- If a helper exists, it should live in ACP SDK land or in docs/examples, not as a Fireline ACP wrapper.

### 4. Is `createFirelineDB` usage clean?

Yes.

What the app does:
- Instantiates the DB from `handle.state.url`
- Calls `preload()`
- Closes on cleanup
- Uses `useLiveQuery` directly over collections

Why this feels right:
- The observation plane is explicit and separate from ACP.
- The React lifecycle code is modest and understandable.
- The query code reads like normal TanStack DB usage, not custom Fireline glue.

Verdict:
- `LEGITIMATE`

### 5. Other patterns where the app is fighting the API

Only one notable issue:
- The fixed header text and fixed test assumptions around `fireline-harness-state` over-couple the UI to a chosen stream name even though the actual runtime handle already exposes the authoritative observation URL.

Verdict:
- `APP BUG` (minor)

## Overall judgment

The browser harness is mostly aligned with the intended three-plane architecture:
- Fireline control plane for provisioning/admin
- ACP SDK directly for session traffic
- `@fireline/state` for observation

The main pain point is ACP ceremony, not Fireline control-plane ergonomics. That pain should be addressed with:
- docs/snippets, and possibly
- a small ACP-SDK-side helper for default client handlers / WebSocket transport setup

It should **not** be addressed by reintroducing a Fireline ACP wrapper.
