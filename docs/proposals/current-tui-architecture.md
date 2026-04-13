# Current TUI Architecture

> Status: current-state architecture note
> Audience: engineers extending the current Fireline Ink REPL and needing the rationale behind the shipped layout
> Context: preserved from the earlier incorrect `mono-thnc.15` scope so the current REPL design remains documented even though `mono-thnc.15` itself was corrected to a future-state redesign proposal

The Fireline REPL is not trying to be a general terminal multiplexer.

It is trying to make one running Fireline session legible and controllable from a terminal:

- show the transcript as it forms
- surface blocking approval work at the moment it matters
- keep one input affordance pinned at the bottom
- degrade safely when the environment cannot support the full Ink experience

That is the design.

## The Problem This Solves

The Fireline CLI already had the hard parts of a REPL path:

- connect to a running ACP endpoint
- create or resume a session
- send prompts
- receive `session_update` events

What it lacked was a terminal surface that could make those updates readable during real work.

A plain stream of lines is enough for a toy demo. It breaks down quickly once the session has:

- streamed assistant text
- tool calls changing status over time
- plan updates
- approval requests that block tool execution

The TUI exists because the REPL is not just a prompt loop. It is a live view over a running session.

## The Core Layout

The REPL is organized around three user-facing regions:

1. transcript region
2. approval prompt region
3. input line

There is also a lightweight status header above them, but the mental model should stay simple: history, blocking action, and composition.

Text diagram:

```text
┌──────────────────────────────────────────────┐
│ Fireline REPL / session / host / usage      │  status header
├──────────────────────────────────────────────┤
│ transcript history                          │
│ assistant messages                          │
│ tool cards                                  │
│ plan cards                                  │
│ current live turn                           │
├──────────────────────────────────────────────┤
│ approval pending: y / n                     │  only when a permission request is pending
├──────────────────────────────────────────────┤
│ > input line                                │  pinned composer
└──────────────────────────────────────────────┘
```

That layout is intentional.

The transcript stays visually dominant because the REPL is first a session viewer.
The approval prompt appears only when the session is actually blocked.
The composer stays pinned because the user always needs to know where new input goes.

This is a stronger constraint than it may look at first glance.

Many terminal interfaces drift toward one of two failure modes:

- everything becomes one undifferentiated output stream
- every state change becomes its own panel until the screen is mostly chrome

The Fireline REPL is intentionally avoiding both.

The transcript must remain the center of gravity because that is where the session's meaning lives.
The approval region must remain narrow because approvals are interruptions, not the product.
The input line must remain obvious because the terminal needs a single canonical place for new intent.

## Ink Component Decomposition

The component split in [`packages/fireline/src/repl-ui.tsx`](../../packages/fireline/src/repl-ui.tsx) reflects those responsibilities directly.

Top-level shell:

- `FirelineReplApp`

Status and chrome:

- `Header`
- `EmptyState`
- `Composer`
- `ApprovalPrompt`

Transcript rendering:

- `EntryView`
- `MessageView`
- `ToolView`
- `PlanView`

Transcript partitioning:

- `partitionTranscriptEntries`
- `hasActiveTurn`
- `findLiveTurnStart`

Supporting presentation helpers:

- `useSpinner`
- `toolStatusColor`
- `renderUsage`
- `hostLabel`

Input and subscription primitives:

- `useSyncExternalStore`
- `useInput`
- `useApp`

That split matters because the design is deliberately not "one big terminal renderer."

The view tree separates:

- durable session state from local input state
- transcript rendering from interaction handling
- stable history from the currently active turn
- blocking approval UI from ordinary message entry

That is what makes the REPL extensible without turning every new UX tweak into a top-level rewrite.

An engineer extending this surface should be able to answer two questions immediately:

1. Is this a new kind of session state, or only a new rendering of existing state?
2. Does this belong in the transcript, the blocking-action lane, or the input lane?

The current component decomposition makes those decisions local instead of architectural.

### A Practical Component Hierarchy

At a high level, the live tree is shaped like this:

```text
FirelineReplApp
  Header
  Static(committed transcript)
    EntryView
      MessageView | ToolView | PlanView
  ApprovalPrompt?          only when permission work is pending
  Live transcript region   current active turn only
    EntryView
  Composer
```

That hierarchy is doing real UX work.

`Header` carries compact session metadata that should remain visible without competing with the transcript.
`<Static>` turns settled history into ordinary terminal scrollback.
`ApprovalPrompt` takes over only when the session is actually blocked.
The live transcript region keeps the currently changing turn near the composer, which is where the user's attention already is.
`Composer` stays last so the interaction point is stable even as the rest of the session changes.

The result is that the REPL can stream aggressively without feeling visually unstable.

## The State Boundary

The REPL's architecture works because `repl.ts` keeps the stateful session and transport concerns separate from Ink presentation.

[`packages/fireline/src/repl.ts`](../../packages/fireline/src/repl.ts) owns:

- ACP connection and session bootstrap
- prompt submission
- session-update ingestion
- approval watching through `fireline.db({ stateStreamUrl })`
- approval resolution through `appendApprovalResolved(...)`
- the `ReplController` view-model
- Ink startup and the terminal capability boundary where fallback logic belongs

`repl-ui.tsx` owns:

- keyboard interaction at the terminal edge
- layout
- message, tool, and plan presentation
- approval prompt rendering
- transcript partitioning into committed and live regions

That separation is the main reason the TUI is viable.

It means the UI is sitting on top of a controller that can also power:

- a plain line-mode fallback
- future tests that do not need a real TTY
- later visual refinements without touching ACP transport logic

The controller boundary also explains why the REPL can evolve without rewriting the UX from scratch.

If approval state changes shape, that change belongs in `ReplController`.
If plan events get a richer presentation, that belongs in `PlanView`.
If a non-TTY execution path is added later, it should still consume the same controller snapshot instead of inventing a second session model.

The architecture is not "React all the way down." It is a session controller with one primary terminal renderer layered on top.

## Why Ink Is The Primary UI

Ink is the right primary surface because the REPL is not just printing finished lines.

It needs live terminal behavior:

- key-by-key input
- a pinned composer
- compact status boxes
- live tool status updates
- approval interception without losing the transcript context

Those are all naturally modeled as terminal UI components rather than as ad hoc console writes.

Ink also matches the structure of the current REPL state model:

- one subscribed snapshot
- a small set of renderable entry types
- a narrow set of user actions

That makes the UI declarative in the same way the surrounding Fireline surfaces already are.

There is also a more practical reason to prefer Ink over handcrafted cursor control.

The REPL needs to combine:

- a continuously changing transcript
- conditional UI that appears only when the workflow is blocked
- status metadata that should stay visible
- a text composer that remains stable while everything else updates

That is exactly the kind of composition problem Ink is good at.

The alternative is not "simpler terminal output." The alternative is a large amount of bespoke terminal bookkeeping for focus, rerendering, and layout correctness. That would create more surface area while producing a less extensible design.

## Why Ink Requires A Raw-Mode TTY

The Ink path depends on terminal capabilities that ordinary piped stdin/stdout do not provide.

In particular, the REPL needs:

- a real TTY on input
- a real TTY on output
- raw keyboard input so Ink can react to single-key decisions such as `y` / `n`, `Esc`, `Ctrl+C`, and `Ctrl+D`
- optional alternate-screen behavior for a focused full-screen session

Without raw-mode terminal control, the Ink path cannot safely promise the interaction model it is designed around.

That is why the TUI should be treated as the primary interactive mode, not as the only mode.

The requirement is not incidental to one approval shortcut.

Raw mode is what lets the REPL distinguish among:

- ordinary text composition
- escape sequences
- approval decisions that should resolve immediately
- shell-like termination keys

If Fireline cannot receive those signals directly, the REPL should stop pretending it can offer the same UX.

## Line-Mode Fallback Contract

The compatibility rule is straightforward:

When Fireline cannot rely on a raw-mode TTY, it should fall back to plain line mode instead of pretending the Ink UI is available.

That fallback path should preserve the same session semantics:

- same ACP connection
- same `ReplController`
- same approval watcher
- same `/quit` and teardown contract

What changes is only the terminal surface:

- one line entered at a time instead of key-by-key input
- approval prompt rendered as plain terminal text
- no alternate-screen layout
- no attempt to keep a pinned live composer

This is an architectural boundary, not a cosmetic preference. The REPL should never make terminal capability assumptions it cannot verify.

This fallback matters for three reasons.

First, Fireline should remain operable in automation-adjacent environments where a real interactive terminal is not available.

Second, line mode is the safe answer when terminal capabilities are ambiguous. A degraded but correct REPL is better than an interface that half-enters raw mode and produces surprising behavior.

Third, the fallback keeps the session model honest. If the same controller can power both modes, the UI remains a presentation choice rather than a second execution path.

Recommended environment detection for the primary path:

- `stdin.isTTY === true`
- `stdout.isTTY === true`
- the input stream can enter raw mode
- the session is not intentionally forced into plain mode by a higher-level CLI choice

In practice, that means the CLI should answer two separate questions before taking the Ink path:

1. Is this environment interactive enough to support key-driven terminal UI?
2. Does the caller actually want the alternate-screen experience that Ink provides?

Those questions are related, but they are not identical. A terminal can technically support Ink while a higher-level caller still prefers plain line mode for logging, demos, automation wrappers, or remote shells.

Recommended fallback triggers:

- piped stdin
- redirected stdout
- non-interactive CI
- terminals that cannot safely enter raw mode
- environments where alternate-screen behavior is undesirable or unsupported

The important point is not the exact heuristic. The important point is that the TTY contract is explicit.

The current landed surface is Ink-first. This RFC is describing the intended compatibility boundary around that surface, so future line-mode work does not drift into an accidental second REPL.

## Approval Prompt Semantics

The approval surface that landed under `mono-thnc.13` is intentionally narrow:

- watch `db.permissions`
- select the earliest pending approval for the current session
- render one blocking approval card
- resolve it with `y` or `n`

The actual data path in `repl.ts` is:

1. `runRepl()` creates an approval watcher when `stateStreamUrl` is available.
2. `watchApprovals()` opens `fireline.db({ stateStreamUrl })`.
3. `db.permissions.subscribe(...)` feeds rows into `selectPendingApproval(...)`.
4. `selectPendingApproval(...)` filters by current `sessionId`, `state === 'pending'`, and a valid request id, then chooses the earliest pending row by `createdAt`.
5. `ReplController.setPendingApproval(...)` moves that state into the UI snapshot.
6. `FirelineReplApp` switches input handling into approval mode.
7. `y` or `n` calls `resolvePendingApproval(true | false)`.
8. `ReplController` delegates to `appendApprovalResolved(...)` with `resolvedBy: 'cli-repl'`.

That design is intentionally conservative.

The REPL does not try to be a full approval dashboard. It handles the one thing the user actually needs in the middle of a blocked interactive session: answer the pending approval and continue.

The subscription point matters here.

Approval state is not inferred from speculative UI state or from prompt text. It is read from `db.permissions`, which means the REPL is reacting to the same durable workflow state the rest of the system can observe.

That gives the approval card three important properties:

- it survives process boundaries because the state is not local to the Ink tree
- it can be rendered by any compatible client that watches the same state stream
- it resolves by appending a real workflow event rather than by mutating UI-local state

That is why the approval prompt belongs in this REPL even though the REPL itself is "just a terminal client." The terminal is presenting durable session state, not inventing it.

### Why `y` / `n` Is Correct

Single-key approval handling is not just convenience.

It matches the severity and timing of the interaction:

- the session is already blocked
- the user is already focused on the terminal
- the next required action is binary

Anything more elaborate would add cognitive overhead at the exact moment the REPL should be unblocking the conversation.

The current semantics deliberately exclude:

- batching
- queue management
- policy editing
- multi-approval review workflows

Those belong in richer operator surfaces, not in the minimal interactive REPL.

There is a second design choice hidden here: only one approval is foregrounded at a time.

That is not a scalability claim. It is a UX claim.

An interactive terminal session needs one blocking question, one answer, and a return to the transcript. As soon as the REPL starts presenting approval queues as a first-class browsing experience, it stops being a REPL and starts becoming an operations console.

## Transcript Model

The transcript is not one flat text blob.

The REPL renders three entry classes:

- messages
- tool events
- plans

This matches the session-update shapes the controller actually receives:

- streamed message chunks become `MessageEntry`
- tool call and tool update notifications become `ToolEntry`
- plan notifications become `PlanEntry`

That decomposition matters because the TUI is supposed to explain a running session, not just dump transport data. Tool work and plan updates need visual distinction from assistant prose or the interface stops being useful once the session does real work.

It also keeps future growth manageable.

If Fireline later adds a new category of durable session event, the design question is not "how do we wedge this into a text stream?" The design question is "is this a new entry kind, or is it a refinement of an existing one?"

That is a much healthier boundary for a terminal interface that is expected to keep evolving.

## Live Turn Versus Committed History

One of the most important REPL design choices is the split between committed history and the active turn.

`partitionTranscriptEntries(...)` draws that line with a simple rule:

- if there is no active turn, all entries are committed
- if there is active work, the transcript is split at the last user message

The result is:

- older turns are treated as stable history
- the current turn remains in the live region until it settles

That matches how a terminal REPL should feel. The active turn is still changing, so it belongs near the composer. Older turns should become ordinary history.

This split is also why the REPL remains readable when a turn contains multiple tool updates.

During active work, the user needs to see the changing portion of the conversation without losing their anchor point.
Once the turn settles, the terminal should stop spending reconciliation work on it and let it become plain scrollback.

The design is therefore temporal as much as visual:

- live work stays close
- finished work becomes history
- the boundary is based on workflow activity, not on arbitrary screen size

## Native Scrollback Matters

This design pressure was captured in the queued `mono-nly` follow-on: the terminal's native scrollback is a feature, not an implementation detail.

Users expect to be able to:

- scroll up with the terminal
- select text across many lines
- copy and paste directly from history
- use terminal or tmux scrollback tools
- search prior output with the tools their terminal already provides

A REPL that traps all history inside a constantly re-rendered live tree breaks those expectations even if the pixels look fine.

That is why the transcript architecture must preserve native scrollback semantics as the REPL evolves.

This matters especially for the Fireline audience, which often runs inside:

- tmux or screen
- remote shells over SSH
- CI-adjacent sandboxes
- terminals with strong built-in search and copy workflows

If the REPL forces those users into a fake scrollback experience, it is fighting the environment instead of composing with it.

## `<Static>` Scrollback Direction

The current REPL already reflects the direction that `mono-nly` argued for:

- committed transcript entries are rendered through Ink `<Static>`
- only the active turn remains in the live render tree

That is the right long-term invariant.

The RFC does not need to duplicate the full refactor analysis. The important design point is simpler:

Committed history should leave Ink's live reconciliation path as early as possible so the terminal can own it as real scrollback.

Future TUI work should preserve that rule even if the exact component boundaries change.

The deeper reason is that terminal interfaces feel better when they stop touching old output.

Once a turn is done, users expect:

- page-up to work normally
- text selection not to fight rerenders
- copied text to stay stable
- terminal logging and capture tools to see settled output as settled output

`<Static>` is useful here not because it is fashionable, but because it matches the user's mental model of terminal history.

This is the correct direction even if the exact implementation continues to evolve.

## Header And Status Design

The header is intentionally compact.

It summarizes:

- the REPL identity
- session id
- current host label
- pending tool count
- usage information
- whether the session is idle, live, or currently resolving approval

That is enough context to answer the questions a user asks repeatedly while working:

- which session am I looking at?
- is work still running?
- am I waiting on tools, approval, or neither?
- am I still connected to the environment I think I am?

The header should not become a dashboard.

If a piece of information is not needed continuously while reading the transcript, it probably does not belong in the always-visible top strip.

## Input Semantics

The composer is simple on purpose.

Normal behavior:

- printable text appends to the current line
- `Enter` submits
- `Esc` clears the draft
- `Backspace` and `Delete` edit the draft
- `Ctrl+C` exits with a shell-friendly interrupt code
- `Ctrl+D` exits cleanly when the draft is empty

Approval behavior:

- printable `y` resolves allow
- printable `n` resolves deny
- other text entry is ignored until the approval is answered

Busy behavior:

- the REPL preserves the transcript and status context
- the composer stops acting like a second command queue

That last point is important.

The REPL is not trying to buffer speculative prompts while a turn is still active. The interface is choosing clarity over throughput.

For an interactive workflow surface, that is the correct trade.

## Alternate Screen And Focus

`runInkRepl(...)` starts the TUI in alternate-screen mode for a reason.

The REPL is meant to feel like a focused working surface while it is active:

- the session gets a clean visual boundary
- the pinned composer does not compete with surrounding shell output
- dynamic rerendering does not smear across unrelated terminal history

At the same time, alternate-screen behavior is a terminal capability choice, not a law of the architecture.

That is another reason the line-mode boundary matters. If a caller or environment does not want alternate-screen interaction, the system needs a compatible lower-fidelity path rather than a broken approximation of the Ink UI.

## UX Invariants

Any extension to the REPL should preserve these invariants:

1. The composer stays visually pinned at the bottom of the interactive region.
2. Approval prompts preempt ordinary text entry without hiding transcript context.
3. The session transcript distinguishes user, assistant, tool, and plan information clearly.
4. The active turn is visually separate from committed history.
5. Committed history remains compatible with native terminal scrollback.
6. Terminal capability assumptions stay explicit; no raw-mode TTY, no forced Ink path.
7. Session semantics stay identical across Ink mode and any line-mode fallback.

These invariants matter more than any individual border color, status label, or spacing choice.

There are also two negative invariants worth keeping explicit:

1. The REPL should not become a window manager for many concurrent concerns.
2. The REPL should not hide durable state transitions behind UI-only shorthand.

Those constraints are what keep the interface small without making it fragile.

## How To Extend The TUI Safely

If you add a new surface to the REPL, place it by asking which layer it belongs to.

Controller-layer additions:

- new session-update entry types
- derived view state
- approval selection policy
- additional usage or status signals

UI-layer additions:

- rendering for a new entry kind
- small status affordances
- richer input hints
- alternate visual treatments that do not change the controller contract

Fallback-layer additions:

- line-mode rendering of the same state
- capability checks
- non-TTY or automation-safe behavior

What should not happen:

- transport logic drifting into Ink components
- terminal capability assumptions drifting into the controller
- approval semantics forking between the Ink path and fallback path

Good extensions generally have one of these shapes:

- a new transcript card for a real new session event
- a better summary of state the controller already knows
- a new fallback rendering of the same controller snapshot

Risky extensions usually have one of these shapes:

- local UI state that can disagree with durable workflow state
- new keyboard behavior that silently changes the approval contract
- additional panes that compete with the transcript for attention
- separate controller logic for Ink and non-Ink modes

The rule of thumb is simple: extend the explanation power of the REPL without multiplying its sources of truth.

## Relationship To Durable Subscribers And Durable Promises

This REPL surface sits directly on top of the workflow substrate story:

- approvals are the passive durable-subscriber case made interactive
- the pending approval card is a terminal presentation of durable workflow state
- the eventual durable-promises / awakeable story uses the same underlying wait-and-resolve model

That is why this RFC pairs naturally with:

- [RFC: Durable Subscribers](./durable-subscriber.md)
- [RFC: Durable Promises](./durable-promises.md)

The REPL is one of the clearest places where the abstract workflow model becomes visible to a user.

The approval prompt is especially useful as a concrete example.

At the workflow layer, approval is durable suspended work waiting on an external decision.
At the REPL layer, that same state becomes a blocking card with a binary keypress decision.

That translation is exactly the kind of user-facing architecture rationale an RFC should make legible.

## References

- [packages/fireline/src/repl-ui.tsx](../../packages/fireline/src/repl-ui.tsx)
- [packages/fireline/src/repl.ts](../../packages/fireline/src/repl.ts)
- [packages/fireline/src/repl.test.ts](../../packages/fireline/src/repl.test.ts)
- [RFC: Durable Subscribers](./durable-subscriber.md)
- [RFC: Durable Promises](./durable-promises.md)
