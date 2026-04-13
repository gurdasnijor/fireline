# TUI Redesign

> Status: proposal
> Date: 2026-04-13
> Scope: future-state multi-pane Fireline terminal UI for live conversation, materialized state inspection, and operator control
> Visual reference: Claude Agent SDK screenshot as interaction inspiration only, not as an API or architecture dependency

## TL;DR

The current Fireline REPL is a good single-thread terminal chat surface.

It is not yet a good operator console.

The redesign in this document replaces the single-column "chat plus prompt" layout with a three-pane terminal UI:

1. **Conversation pane** for the active session transcript and streaming reply.
2. **Materialized state pane** for the current session's durable state rendered as cards and compact tables.
3. **Realtime event pane** for raw ACP traffic, durable-state appends, and control-plane lifecycle events.

The point of the redesign is not cosmetics.

It is to let one terminal answer three different questions at once:

- what is the agent saying right now?
- what durable state exists right now?
- what raw events are moving right now?

Those questions should not share one undifferentiated transcript.

The proposal keeps Ink as the primary renderer for the first implementation pass.
The repository already depends on `ink` `^7.0.0` in `packages/fireline/package.json`, so the practical feasibility question is not "should Fireline move to Ink 5?" but "is the current Ink-based stack sufficient for a richer dashboard, or does the redesign force a framework swap?"

My recommendation is:

- **stay on Ink** for the first redesign cut
- keep **keyboard-first interaction** as the guaranteed UX
- treat **mouseable approval buttons** as a stretch layer unless Fireline is willing to own lower-level terminal input handling or a later renderer adapter

## Why A Redesign Exists At All

The current REPL in [`packages/fireline/src/repl-ui.tsx`](../../packages/fireline/src/repl-ui.tsx) and [`packages/fireline/src/repl.ts`](../../packages/fireline/src/repl.ts) solves the narrow problem well:

- open or resume a session
- stream `session_update` messages
- show tool and plan activity inline
- surface one pending approval
- keep a single composer pinned at the bottom

That is the right MVP.

It is not the right long-term operator surface because the current layout collapses three fundamentally different viewpoints into one lane:

- the conversational transcript
- the durable state model projected from the stream
- the underlying event traffic and lifecycle churn

When those are mixed together, the terminal stops being explanatory.

An engineer trying to debug or operate a live Fireline session usually needs all of the following simultaneously:

- the latest assistant reply
- the current approval state
- the latest projected permission rows
- the prompt/request lifecycle for the selected session
- whether the runtime is healthy, idle, or broken
- whether the host just restarted or the session was reloaded
- the raw ACP or durable-state event that explains why the UI changed

One transcript cannot carry all of that without becoming noise.

## Non-Goals

This proposal is not trying to turn `fireline repl` into:

- a general terminal multiplexer
- a browser dashboard squeezed into ANSI
- a full trace UI
- a replacement for external observability backends
- a justification for violating Fireline's plane-separation rules

The terminal remains a focused operator tool.

It should be excellent at one live session, legible across several concurrent sessions, and honest about what belongs in Fireline state versus what belongs in traces or admin/control surfaces.

## Design Principle: Three Questions, Three Panes

The redesign is organized around a simple rule:

> Each pane answers one question well, and the panes do not compete for the same semantic job.

### Pane 1: Conversation

Question:

> What is the selected session saying and doing right now?

This pane owns:

- user prompts
- agent reply chunks
- plan updates
- tool progress as conversation-adjacent cards
- inline approval cards at the point where the session blocks

It is the narrative surface.

### Pane 2: Materialized State

Question:

> What durable state exists for the selected session right now?

This pane owns:

- session summary
- prompt request lifecycle
- approval history and current pending state
- chunk-derived summaries
- derived "tools in flight" and "latest plan" summaries
- operator cards about the runtime and launch configuration when that data exists on the operator plane

It is the read model.

### Pane 3: Realtime Events

Question:

> What raw events are flowing through the system right now?

This pane owns:

- ACP wire events such as `session/new`, `session/prompt`, `session_update`, `session/load`
- durable-state append events by collection
- control-plane or host lifecycle events such as provision, boot, teardown, and runtime death

It is the causality and debugging surface.

## Why Three Panes Instead Of One Timeline

A single event timeline sounds simpler on paper.

In practice it fails because the three data classes above have different stability and different intended use:

- conversation is human-readable narrative
- projected state is durable and query-shaped
- raw events are noisy but useful for causality and debugging

If Fireline puts all three in one stream:

- durable state rows get lost in transcript prose
- raw transport chatter drowns out the conversation
- the user has to mentally reconstruct which entries are facts and which are merely observations of facts

Three panes avoid that confusion by making the semantic split visible.

The redesign is therefore not "more panels because screenshots look modern."

It is a stricter explanation model.

## Proposed Layout

The default layout should be a wide left conversation column plus a split right utility column.

Text diagram:

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│ Fireline TUI  session tabs  host/runtime status  filters  shortcuts         │
├──────────────────────────────────────┬───────────────────────────────────────┤
│ Pane 1: Conversation                 │ Pane 2: Materialized session state   │
│                                      │                                       │
│ transcript                           │ session card                          │
│ streamed reply                       │ prompt request table                  │
│ tool cards                           │ approval cards/history                │
│ plan cards                           │ tools/memory/context cards            │
│ inline approval card                 │ runtime + spawn/admin cards           │
│ composer                             │                                       │
├──────────────────────────────────────┼───────────────────────────────────────┤
│ Pane 1 continues                     │ Pane 3: Realtime events              │
│                                      │                                       │
│                                      │ ACP events                            │
│                                      │ durable append events                 │
│                                      │ host/control events                   │
│                                      │ filters + tail state                  │
└──────────────────────────────────────┴───────────────────────────────────────┘
```

Default proportions:

- left conversation column: roughly 58-65% width
- right column: roughly 35-42% width
- within the right column, state pane gets more height than events by default

Why this cut is correct:

- conversation needs the most width because text wraps poorly in narrow columns
- state cards need enough width for ids, timestamps, and small JSON snippets
- event lines are naturally denser and tolerate a narrower lane better than prose

### Narrow-Terminal Behavior

On narrow terminals, the layout should not attempt a comically thin three-column squeeze.

Instead:

- keep a compact top bar
- switch the right-hand utility area to tabbed or hotkey-selected subviews
- preserve the conversation pane as the stable anchor

The terminal must never degrade into unreadable micro-columns simply to preserve a desktop-like screenshot.

## Pane 1: Conversation Design

Pane 1 inherits the best part of the current REPL:

- it is the primary reading and writing surface
- it keeps the composer visually stable
- it shows work as it happens rather than only after the turn completes

What changes is not the purpose.

What changes is the amount of contextual structure around it.

### Pane 1 Responsibilities

Pane 1 should render:

- prompt messages
- assistant streaming text
- thought or reasoning text when surfaced
- tool activity cards
- plan cards
- inline approval cards
- load/resume markers when the selected session is reattached

It should not render:

- raw ACP envelopes
- durable append metadata
- host boot or control-plane chatter unless that event is directly narratively relevant

### Streaming Model

The current controller model in `repl.ts` is still the correct basis:

- ACP `session_update` notifications are the lowest-latency source for live reply text
- the view model coalesces chunks into entries
- tool and plan events become typed render units instead of raw JSON blobs

That should remain true in the redesign.

Pane 1 is the one place where Fireline should optimize for immediacy over exhaustive durability detail.

The conversation must feel live first.

Pane 2 and pane 3 exist specifically so pane 1 does not have to carry every implementation detail.

### Scrollback And `mono-nly`

The `mono-nly` follow-on captured an important terminal truth:

> Native scrollback is a feature, not an implementation accident.

Users expect to:

- page up through prior output
- select text directly in the terminal
- copy long reply spans
- search history with terminal or tmux tooling

That remains true after the redesign.

The correct invariant is:

- **committed conversation history should leave Ink's hot render path as early as possible**
- **the active turn and live utility panes can remain in the live render tree**

This does create a design consequence for the three-pane shell:

- historic conversation will be the durable artifact that enters terminal scrollback
- the right-hand state and event panes are live operator surfaces, not historical screenshots of every prior moment

That is acceptable.

When a user scrolls back, they are usually trying to reread the conversation.

They are not trying to recover an exact pixel-perfect reconstruction of the utility panes from twelve minutes ago.

### Composer Placement

The composer remains at the bottom of pane 1.

That invariant survives the redesign because the terminal needs one unambiguous place for new intent.

Even when the right-hand panes are active and changing, prompt composition should not visually drift.

## Pane 2: Materialized Session State

Pane 2 is the most important change in the redesign.

It turns Fireline's durable read model into a first-class operator surface instead of a hidden implementation detail behind `fireline.db()`.

### Pane 2 Purpose

Pane 2 should answer:

- what request is active?
- what requests already completed?
- is approval pending, resolved, or orphaned?
- what tool activity does durable state currently imply?
- what session am I attached to?
- what runtime or launch context is this session living on?

This pane is intentionally card- and table-oriented.

It should feel like a compact, inspectable control room rather than a second transcript.

### Pane 2 Data Model

The current public `fireline.db()` surface exposes four live collections:

- `db.sessions`
- `db.promptRequests`
- `db.permissions`
- `db.chunks`

These are enough to build most of the session-state experience already:

- session card from `db.sessions`
- request timeline from `db.promptRequests`
- approval queue and history from `db.permissions`
- tool/status/context summaries from `db.chunks`

Derived views already exist or are straightforward:

- session-specific request collection
- session-specific permission collection
- request-specific chunk collection
- pending-permission collection

That is the right substrate for pane 2.

### Pane 2 Must Stay A Read Model, Not A Log Viewer

Pane 2 should not show every append event.

It should show the **current materialized truth** for the selected session.

That distinction matters.

If pane 2 becomes an event log, it duplicates pane 3 and loses the main benefit of a projected state surface:

- rows collapse repeated churn into a stable current answer
- tables and cards show the latest known state without making the user read the full event history

Pane 2 is where Fireline proves that the stream is useful as a state substrate, not just as an audit log.

### The Plane-Separation Constraint

This proposal needs to be explicit about one tension.

The requested pane 2 wants to show runtime state and spawn config near the current session.

That is a good operator need.

It does **not** justify stuffing infrastructure rows back into `fireline.db()`.

Earlier design work intentionally moved the public DB surface toward agent-plane state only.

That rule should stand.

So the correct design is:

- **visually unify** session-state cards and operator/runtime cards in one pane
- **do not pretend** they come from the same underlying plane

Concretely:

- agent/session facts come from `fireline.db()`
- runtime lifecycle and spawn/config facts come from the admin/operator plane

If Fireline later wants a unified operator read model, it should create an explicit admin/operator subscription surface rather than quietly re-polluting `fireline.db()`.

### Pane 2 Card Taxonomy

The default right-hand state pane should include these card groups.

#### Session Card

Shows:

- selected `sessionId`
- session state
- last-seen timestamp
- whether load/resume is supported
- current host or ACP endpoint label

Primary source:

- `db.sessions.subscribe(...)`

#### Request Timeline Card

Shows:

- queued
- active
- completed
- timed out
- broken
- cancelled

Primary source:

- `db.promptRequests.subscribe(...)`, filtered to the selected session

This is the durable explanation for what the conversation lane is doing.

#### Approval Card Stack

Shows:

- current pending approval
- latest resolved approvals
- outcome badges
- request id, tool call id, and timestamps

Primary source:

- `db.permissions.subscribe(...)`, filtered to the selected session

This pane mirrors pane 1's inline approval card, but it renders approval as state, not as interruption.

#### Tool / Context Card

Shows derived summaries such as:

- tools currently in progress
- last completed tool
- latest plan summary
- recent chunk-derived status snippets

Primary source:

- `db.chunks.subscribe(...)`, filtered to the selected session and grouped by request or tool call id

This card is intentionally derived.

Fireline should not invent a fake `tools` collection merely to make the TUI easier to render if the existing chunk substrate is already sufficient.

#### Runtime / Launch Card

Shows:

- sandbox id
- provider
- status
- launch/runtime metadata
- relevant labels
- spawn or harness summary

Primary source:

- admin/operator subscription surface or polling wrapper, not the current public `fireline.db()`

This card belongs in the pane because operators need it.

It does not belong in the agent-plane DB.

#### Topology / Peer Card

Optional when relevant:

- peer names
- supervisor/worker relationship
- harness name
- multi-agent topology summary

Primary sources:

- harness metadata
- session selection context
- future operator topology descriptors where available

### Pane 2 Data-Source Mapping

| Pane 2 region | Primary source | Derivation rule | Why it belongs here |
|---|---|---|---|
| Session summary | `db.sessions.subscribe(...)` | selected session row | durable session truth |
| Request timeline | `db.promptRequests.subscribe(...)` | session filter, sort by `startedAt` | request lifecycle is state, not transcript |
| Approval stack | `db.permissions.subscribe(...)` | session filter, split pending vs resolved | approval is durable state first, prompt interruption second |
| Tool/context summary | `db.chunks.subscribe(...)` | group by request / tool call id and summarize latest status | chunk projection explains live work without raw event noise |
| Runtime/launch card | admin/operator subscription | join selected session to runtime descriptor | operator need, but not agent-plane DB |
| Session-switch badges | session rows plus operator metadata | aggregate across open sessions | supports concurrent session navigation |

## Pane 3: Realtime Event Stream

Pane 3 is the anti-confusion pane.

It exists so the user can answer:

> What just happened in the underlying system that caused the other panes to change?

Pane 3 is not the durable state surface.

Pane 3 is not the chat surface.

Pane 3 is the raw "tail -f" operator lane.

### Pane 3 Event Classes

The pane should merge three event categories:

1. **ACP wire events**
2. **Durable-state append events**
3. **Control-plane / lifecycle events**

#### ACP Wire Events

Examples:

- `initialize`
- `session/new`
- `session/prompt`
- `session_update`
- `session/load`

These should show method, direction, session id, request id when present, and a compact payload summary.

#### Durable-State Append Events

Examples:

- prompt request row append
- permission request append
- approval resolved append
- chunk append
- session row update

These should show collection, key, operation, and compact row summary.

This is the "why did pane 2 change?" stream.

#### Control / Lifecycle Events

Examples:

- host boot
- sandbox provisioned
- sandbox stopped
- runtime broken
- session reattached
- resume attempted

This is the "why did the process boundary move?" stream.

### Pane 3 Filtering

Pane 3 must be filterable because otherwise it will become unreadable quickly.

Required filters:

- selected session only / all sessions
- ACP / state / control event type
- warnings and errors only
- tail-follow on/off
- text filter for request id, session id, tool call id, or method name

### Pane 3 Should Be Scrollable, But Not The Same Way As Pane 1

Pane 1 optimizes for native terminal scrollback because it is the long-form reading artifact.

Pane 3 optimizes for focused inspection of a dense live feed.

So pane 3 can legitimately use an internal focusable, pageable list inside the live TUI, provided that:

- copyable ids and timestamps remain visible
- pausing tail-follow does not destroy context
- filters are obvious

This is one place where internal pane scrolling is acceptable.

The event lane is closer to a log viewer than to a transcript.

### Pane 3 Relationship To Observability

Pane 3 is not a replacement for tracing or an observability backend.

It should show local/live event causality.

Cross-process trace history still belongs to Fireline's observability story and OTLP export path, as described in [`observability-integration.md`](./observability-integration.md) and the user-facing observability RFC later derived from it.

The rule is:

- pane 3 is for immediate operator inspection
- traces remain the durable cross-process observability substrate

## Approval UI Redesign

The approval UX is where the redesign most obviously departs from the current MVP.

The `mono-thnc.13` landing proved the semantics:

- watch the permission projection
- surface the pending approval
- resolve it durably

That semantic core remains correct.

The UI should change substantially.

### Why The Current `y` / `n` Prompt Is Not Enough

The current prompt works for a demo because it proves the durable approval path.

It is too thin for a long-lived operator UI because it hides the information a real operator wants before approving a tool:

- which tool is asking
- full arguments
- the stated reason
- which request and tool call this belongs to
- whether there are related approvals nearby

`y` / `n` is the right emergency control.

It is not the right long-term visual language.

### Proposed Approval Card

The primary approval surface should be an inline card rendered in pane 1 and mirrored in pane 2.

Card anatomy:

- title: `Tool Approval`
- tool name
- request id and tool call id in monospace
- pretty-printed JSON arguments
- stated reason text
- timestamp
- action row with `Accept` and `Decline`

Color behavior:

- pending: amber border / accent
- accepted: green
- declined: red
- resolving/in-flight: blue-dim or muted cyan
- error: red with explicit error text

### Interaction Model

Guaranteed interactions:

- keyboard focus
- `Tab` / `Shift+Tab` across actionable controls
- `Enter` or `Space` on focused button
- direct hotkeys such as `a` for accept and `d` for decline when the approval card is focused

Optional or stretch interaction:

- mouse click on `Accept` / `Decline`

The reason for this split is technical honesty.

Ink's documented strengths are:

- box layout
- text rendering
- keyboard input via `useInput`
- focus management via `useFocus` and `useFocusManager`
- terminal sizing and alternate-screen control

I did **not** find a documented first-party mouse click API in the official Ink README or the release notes reviewed for this proposal.

Inference:

- keyboard-first approval buttons are clearly feasible on the current stack
- mouseable buttons may require lower-level terminal escape handling or a framework adapter Fireline does not currently own

So the proposal should not pretend mouse is free.

### Approval Queue Semantics

The conversation pane should foreground one blocking approval at a time for the selected session.

Pane 2 can still show recent approval history and any secondary pending items if Fireline later supports them.

The terminal does not need a dense queue browser in the center of the conversation flow.

## Visual System

The redesign should look more deliberate than the current border-only REPL, but it still needs terminal discipline.

### Typography Hierarchy

Use four text roles consistently:

- heading
- body
- monospace identifiers
- dim metadata such as timestamps, hostnames, and collection keys

This sounds obvious, but it matters in a dense terminal UI.

Without strong hierarchy:

- ids disappear into prose
- timestamps become noise
- cards blur together

### Card Language

Card types should be visually consistent across panes:

- conversation cards
- state cards
- event lines or event mini-cards
- approval cards
- admin action cards

Consistent meaning matters more than decorative variety.

For example:

- amber always means pending / awaiting decision
- green always means successful completion or allowed resolution
- red always means denied, failed, or broken
- blue-dim means streaming or currently active but not yet terminal

### What "Memory" Means In This TUI

The Claude reference uses a "Memory" visual language.

Fireline should not invent fake memory rows just to mimic that screenshot.

If Fireline shows a memory-like card, it should mean one of:

- explicit context currently attached to the selected session
- mounted resources or static context sources
- derived recent-plan or working-context summary

The TUI should not imply hidden magical memory state that the durable system does not actually expose.

## Session Switching And Concurrent Sessions

The redesign must support more than one live session.

This does not mean rendering many conversations at once.

It means:

- a top session switcher or session tab strip
- badge counts for pending approval, active tools, or broken state
- the currently selected session driving all three panes

Why this matters:

- admins often need to flip between a blocked session and a healthy one
- multi-agent topologies may expose sibling sessions that share an operator's attention
- session load/resume is easier to reason about when the UI makes session selection explicit

The session switcher is therefore shell chrome, not a fourth pane.

## Admin Controls

The new TUI is not just a chat shell.

It needs explicit operator controls.

Required controls:

- load or resume a known session
- switch among active sessions
- stop or kill a runtime
- restart or reprovision a runtime where supported
- reconnect to the selected session after host churn

These controls belong in a dedicated action area, likely inside the top bar or as focused action cards in pane 2.

They should not be hidden behind slash commands alone.

Slash commands can remain as power-user shortcuts.

They should not be the only operator affordance in a supposedly richer TUI.

## Renderer Feasibility: Ink Versus A Framework Swap

The repo's current implementation already uses Ink and React, and the package dependency is already on Ink 7, not Ink 5.

So the real feasibility question is:

> Can Ink render this design cleanly enough for the first implementation pass without making Fireline own too much terminal plumbing?

### What Ink Is Clearly Good Enough For

Based on the current code and the official Ink docs/release notes, Ink is a strong fit for:

- flexbox-style multi-pane layout with `<Box>`
- compact card rendering
- keyboard input routing with `useInput`
- focus traversal with `useFocus` and `useFocusManager`
- alternate-screen rendering
- window-resize awareness with `useWindowSize`
- fixed action bars and split panes

That is enough for:

- the three-pane shell
- tabbed session switching
- focusable approval buttons
- filter controls
- keyboard-driven event inspection

### What Ink Is Riskier For

Ink is weaker or at least less obviously documented for:

- rich mouse interaction
- deeply nested internal scroll regions that all behave like native terminal apps
- perfect historical right-pane reconstruction while also preserving native scrollback

Those are not proof that Ink is wrong.

They are warnings that the design should lean into Ink's strengths instead of asking it to impersonate a full ncurses application with browser-like widgets.

### Recommendation

For the first redesign cut:

- keep Ink
- keep React
- keep alternate-screen mode
- keep conversation history committed out of the hot render path where possible
- keep keyboard as the guaranteed control model

Do **not** switch immediately to:

- `tui-rs` bindings
- a bespoke ANSI renderer
- a terminal framework rewrite

Reason:

- a renderer swap would explode scope before the actual UX model is proven
- the hardest product questions here are semantic, not graphical
- Ink already covers the core shell, focus, layout, and keyboard model well enough to validate the redesign

The one explicit caveat is mouse.

If product later decides true click support is mandatory for the approval cards, that should trigger a focused renderer-boundary review rather than being smuggled into phase 1 as an assumption.

## Why This Proposal Still Fits Fireline's Architecture Story

The redesign does not change Fireline's substrate story.

It makes it visible.

Pane 1 shows the active ACP conversation.
Pane 2 shows the durable read model projected from the stream.
Pane 3 shows the raw events and lifecycle churn that explain both.

The approval card is especially important because it makes a core Fireline idea obvious:

- the wait is durable
- the resolution is durable
- the UI is a terminal presentation of that substrate, not a fake local prompt state

That same reasoning is why this proposal naturally pairs with the durable-subscriber and durable-promises work:

- approval is a concrete durable wait
- the selected session's state pane is a readable materialization of that wait
- the event pane shows the transport and control churn around it

## UX Invariants

Any implementation that claims to satisfy this proposal should preserve these invariants:

1. Conversation remains the primary reading surface.
2. Projected state and raw events are visible without being dumped into the transcript.
3. The selected session drives all panes.
4. Approval is rendered as a durable workflow decision, not as a transient text prompt.
5. Runtime/admin controls are visible, not hidden behind undocumented commands.
6. Agent-plane state and admin-plane state remain conceptually separate even when composed in one visual pane.
7. Conversation history preserves native terminal scrollback semantics as much as practical.
8. Keyboard operation is first-class and complete.
9. Mouse interaction, if added, is an enhancement rather than a hidden dependency.
10. Narrow terminals degrade by switching views, not by compressing three panes into unreadable slivers.

## Implementation Beads Dropped From This Proposal

This proposal should spin out the following sibling implementation beads under `mono-thnc`:

1. **Conversation pane shell and scrollback invariants**
   - Refactor the live shell so pane 1 can stream current work while committed conversation remains scrollback-friendly.

2. **Session-state pane and derived live queries**
   - Build pane 2 cards and tables from `fireline.db()` plus explicit session-scoped derived queries.

3. **Realtime event pane and ingest adapters**
   - Add ACP wire-event capture, durable append-event feed, filter model, and tail/scroll behavior.

4. **Approval card kit and action model**
   - Replace the MVP `y` / `n` prompt with focusable approval cards, pretty JSON args, and accept/decline actions.

5. **Admin controls and session switcher**
   - Add session tabs, load/resume actions, runtime stop/restart controls, and selected-session coordination across panes.

These are deliberately separated because they touch different risk areas:

- conversation and scrollback behavior
- state projection and read-model design
- event capture and filtering
- approval interaction design
- operator/admin actions

## Open Product Decision

One product decision should stay explicit during implementation:

> Does Fireline want mouse to be a hard requirement for the first shipped redesign?

My recommendation is no.

Ship keyboard-first, visually clear approval and admin controls first.

If the team later wants clickability, add it as a bounded follow-on after the semantic shell is proven.

That is a much safer order than choosing a new renderer prematurely.

## References

- [packages/fireline/src/repl-ui.tsx](../../packages/fireline/src/repl-ui.tsx)
- [packages/fireline/src/repl.ts](../../packages/fireline/src/repl.ts)
- [packages/fireline/src/repl.test.ts](../../packages/fireline/src/repl.test.ts)
- [packages/state/src/collection.ts](../../packages/state/src/collection.ts)
- [packages/state/src/schema.ts](../../packages/state/src/schema.ts)
- [packages/state/src/collections/session-turns.ts](../../packages/state/src/collections/session-turns.ts)
- [packages/state/src/collections/session-permissions.ts](../../packages/state/src/collections/session-permissions.ts)
- [packages/state/src/collections/turn-chunks.ts](../../packages/state/src/collections/turn-chunks.ts)
- [packages/client/src/db.ts](../../packages/client/src/db.ts)
- [packages/client/src/admin.ts](../../packages/client/src/admin.ts)
- [docs/proposals/observability-integration.md](./observability-integration.md)
- `mono-thnc.13` approval UI MVP landing
- `mono-nly` scrollback rationale
- [Ink README](https://github.com/vadimdemedes/ink)
- [Ink releases](https://github.com/vadimdemedes/ink/releases)
