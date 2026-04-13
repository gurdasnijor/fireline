# Betterstack saved view — demo dashboard spec (T5.2)

> Status: scaffolded pre-span-emission. Bake against real spans once T4.1
> (`mono-thnc.4.1`) lands and `fireline` starts exporting.
>
> Target: render the pi-acp → OpenClaw demo trace tree coherently enough to
> serve as Pane C for the operator script, and as the Step 7 close.

## Panel layout

Five panels, left-to-right / top-to-bottom, matching the narrative arc of the
operator script:

```
┌──────────────────────────────┬──────────────────────────────┐
│ 1. Session timeline           │ 2. Prompt request latency     │
│    (event count bucketed)     │    (p50/p95/p99 over window)  │
├──────────────────────────────┼──────────────────────────────┤
│ 3. Tool call heatmap          │ 4. Approval timeline          │
│    (span starts, by tool name)│    (requested / resolved)     │
├──────────────────────────────┴──────────────────────────────┤
│ 5. Trace tree for current demo session                       │
│    (Betterstack default trace view, filtered by session_id)  │
└──────────────────────────────────────────────────────────────┘
```

## Panel specs

### 1. Session timeline

- Span: `fireline.session.created`
- Visualization: time-series bar chart, 15-second buckets
- Y-axis: span count
- Group by: `fireline.session_id`
- Filter: span start within operator-selected window
- Expected during demo: 1–2 bars (one per demo session)

### 2. Prompt request latency

- Span: `fireline.prompt.request`
- Visualization: time-series line, 3 series (p50, p95, p99)
- Y-axis: span duration (ms)
- Computed over: 1-minute rolling window
- Filter: OK-status spans only (exclude OTel error status for clean latency line)
- Expected during demo: flat-ish latency with a bump at Step 3 restart
  (resumed prompt logs a fresh `prompt.request` span under the same
  `session_id`)

### 3. Tool call heatmap

- Span: `fireline.tool.call`
- Visualization: heatmap, X = time (15s buckets), Y = `fireline.tool_name`
- Cell value: span count
- Color scale: linear, capped at max cell count × 1.2
- Expected during demo: thin stripe of `read_file`, `list_files`, maybe
  `delete_file` at Step 4 (which pauses before executing)

### 4. Approval timeline

- Spans: `fireline.approval.requested` + `fireline.approval.resolved`
- Visualization: paired timeline, two rows stacked
  - row A: `requested` spans (orange dot at start)
  - row B: `resolved` spans (green dot if `fireline.allow=true`, red if
    `fireline.allow=false`)
- Tooltip attrs: `fireline.request_id`, `fireline.policy_id`,
  `fireline.reason`, `fireline.resolved_by`
- Expected during demo: 1 requested + 1 resolved pair during Step 4, with
  the gap between them showing the latency from human approval

### 5. Trace tree (demo session)

- Betterstack default trace detail view
- Filter: `fireline.session_id = <current demo session id>`
- Variable: operator binds the session id from Step 1's startup banner into
  a dashboard variable `session_id` (Betterstack template variable feature).
  Before the demo, rehearse: run the flow, note the banner, paste into the
  variable input on the dashboard so Pane C is already bound when curtain
  opens.
- Expected during demo: nested trace with
  `session.created → prompt.request → tool.call*` +
  `approval.requested → approval.resolved`; plus peer.call.out/in if
  T4.2 propagation is live (else two adjacent traces).

## Build order (when T4.1 lands)

1. Drive a real prompt through the locked `docs/demos/assets/agent.ts` with
   Betterstack env sourced.
2. Verify each of the 5 spans arrives by watching
   `curl https://.../query?query=<span-name>` or equivalent.
3. Build panels 1 → 5 in that order; each panel's first draft is "show me
   the span, no aggregation" so we visually confirm the data is there.
4. Apply the aggregations above one panel at a time.
5. Save the view. Capture the saved-view URL under
   `deploy/observability/dashboard/saved-view-url.txt` (gitignored — it
   carries the team + view ID).
6. Reference the URL in `docs/demos/pi-acp-to-openclaw-operator-script.md`
   §Step 7 and pre-flight P8.

## Synthetic-span smoke (before T4.1 lands)

Can start building panels 1–4 against synthetic spans so the layout work
doesn't wait for T4.1. Run `deploy/observability/dashboard/inject-synthetic.sh`
(see sibling file) to push a test trace into Betterstack in the correct
OTLP shape; once dashboards render it, the layout is proven and we swap
the data source the moment real spans arrive.

## Do-not-commit (same as parent README)

- saved-view-url.txt (includes team ID)
- any dashboard export that embeds the source token in headers
