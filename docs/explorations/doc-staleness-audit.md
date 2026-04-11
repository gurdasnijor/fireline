# Doc Staleness Audit

Status: audit only
Date: 2026-04-10
Anchor: [`managed-agents-mapping.md`](./managed-agents-mapping.md)

## Purpose

This audit translates the new anchor doc into concrete doc deltas.

It is intentionally narrow. It focuses on:

- `docs/execution/14-runs-and-sessions-api.md`
- `docs/execution/15-workspace-object.md`
- `docs/execution/16-capability-profiles.md`
- `docs/execution/17-out-of-band-approvals.md`
- `docs/execution/next-steps-proposal.md`
- `docs/runtime/heartbeat-and-registration.md`
- `docs/product/*` files that contradict `docs/product/priorities.md`

This is still audit-only. It does not apply the edits.

## Anchor rules to apply everywhere

1. `Session` is a strong Fireline primitive already. Slice 14 should become a
   **canonical durable read surface**, not a Fireline-owned product object API.
2. `Resources` is the smaller missing primitive. Slice 15 should be demoted
   from a big product-object slice to a **small launch-spec refactor** around
   `ResourceRef` and pluggable mounters.
3. `Orchestration` is the big missing primitive. Add **slice 18** for
   `wake(session_id)` or equivalent runtime wake orchestration.
4. Out-of-band approvals are not their own orchestration primitive. They are a
   **consumer of `wake()`**.
5. `Tools` is already strong. The current capability-profile work should be
   reframed as **portable tool references with `credential_ref` indirection**,
   not a Fireline-owned profile object.
6. Product docs should stop assuming Fireline owns `runs`, `workspaces`,
   `profiles`, or `approvals` as user-facing product systems. Those can exist
   in Flamecast or another consuming product.

## Numbering note

The anchor doc creates a numbering mismatch with the current filenames:

- current `15-workspace-object.md` becomes a smaller Resources refactor
- current `17-out-of-band-approvals.md` maps to the future **slice 16**
  conceptually
- current `16-capability-profiles.md` maps to the future **slice 17**
  conceptually
- new **slice 18** is Orchestration / `wake`

Before editing the content, decide whether to:

- renumber the execution docs to match the new semantics, or
- keep the filenames stable for now and only change titles/body text

The deltas below assume content changes first and filename churn second.

## Execution docs

### `docs/execution/14-runs-and-sessions-api.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Change `## Objective` at lines 16-32.
  Replace "Run as the live managed execution object" and "Session as the durable evidence object" with "Session as Fireline's canonical durable read surface." Remove the sentence about turning them into explicit Fireline product APIs.
- Change `## User Workflow Unlocked` at lines 38-46.
  Make the user here a consuming control plane or UI embedding Fireline state, not a direct Fireline `client.runs` / `client.sessions` consumer.
- Rewrite `## Why This Slice Exists` at lines 48-65.
  Keep the list of existing durable session substrate, but end with "the gap is stable schema and replay/read semantics" rather than "Fireline still looks too much like a runtime toolkit."
- Delete or replace `### 1. Run object projection` at lines 69-96.
  This should become a section like "Canonical session read schema" or "Session read model" and should explicitly say any run object is downstream orchestration state, not Fireline-owned API surface.
- Rewrite `### 2. Session object projection` at lines 98-117.
  Keep `sessionId`, runtime lineage, resumability, and timestamps, but say this is a durable read model over existing rows/events, not a new product object.
- Delete `### 3. Product API surface` at lines 119-143.
  Replace it with a narrower section such as "Read surface shape" or "TS read helpers" that talks about collections, replay, catch-up, and consumer queries rather than `client.runs.*` / `client.sessions.*`.
- Keep `### 4. Durable evidence mapping` at lines 145-158, but rewrite it to anchor on the Session primitive from the managed-agents mapping doc.
  The important message should be "no second hidden event model" and "read contract over the durable stream."
- Rewrite `### 5. Resume and reopen semantics` at lines 160-169.
  Make it about what Session metadata and state are required for a consumer product to perform reopen/resume, not about Fireline directly owning `client.sessions.resume(...)`.
- Rewrite `### 6. First consumer path` at lines 171-182.
  Say the consumer path should be Flamecast/browser/control-plane UI reading Fireline's durable session surface.
- Rewrite `## Acceptance Criteria` at lines 196-208.
  Remove `client.runs.start/get/list` and `client.sessions.get/list/resume/...`; replace with acceptance around canonical row schema, replay/catch-up semantics, lineage/artifact/session lookup, and one consumer proving those reads.
- Rewrite `## Validation` at lines 210-222.
  Replace product-API tests with one TypeScript or UI integration test that consumes the session read surface and one replay/catch-up test.
- Rewrite `## Handoff Note` at lines 224-238.
  End with "Session is Fireline's durable read surface; Run belongs to orchestration."

### `docs/execution/15-workspace-object.md`
Status: stale.
Recommendation: update and demote.

Concrete deltas:

- Change the title and metadata at lines 1-4.
  This should stop being a major execution slice. It should become a short refactor doc for the `Resources` primitive, or be moved out of the numbered slice flow.
- Rewrite `## Objective` at lines 16-33.
  Replace "first real Workspace product object" with "add `resources: [{ source_ref, mount_path }]` to launch specs plus pluggable mounters."
- Change `## Product Pillar` at lines 35-37.
  Replace "Portable workspaces" with "Portable execution inputs" or "Resources."
- Rewrite `## User Workflow Unlocked` at lines 39-46.
  Say products can point Fireline at a local path, git ref, or object-store ref without inventing a heavyweight workspace system.
- Rewrite `## Why This Slice Exists` at lines 48-63.
  Keep the motivation around source context, but explicitly cite the anchor doc's simplification: Resources is a launch-spec field, not a Fireline-owned product object.
- Delete `### 1. Workspace object and identity` at lines 67-85.
  Replace it with `### 1. ResourceRef shape` containing fields like `source_ref`, `mount_path`, and optional mutability or fetch mode.
- Delete `### 2. Product API surface` at lines 87-100.
  Replace it with `### 2. ResourceMounter trait` and name the initial implementations: local path, git remote, S3, GCS.
- Rewrite `### 3. Run and session linkage` at lines 102-112.
  This should become "launch-spec integration" and say `CreateRuntimeSpec` or equivalent carries resources as inputs.
- Delete `### 4. Snapshot as an explicit operation` at lines 113-124.
  Snapshotting may still exist later, but it is no longer the center of this doc.
- Delete `### 5. First-cut reuse semantics` at lines 126-132.
  Replace with "initial mounters and materialization behavior."
- Rewrite `## Acceptance Criteria` at lines 145-154.
  Replace workspace identity and `client.workspaces.*` requirements with `ResourceRef`, `ResourceMounter`, and one proof that a runtime launches with mounted resources.
- Rewrite `## Validation` at lines 156-168.
  Replace workspace product-surface tests with one provider/bootstrap integration test per mounter kind.
- Rewrite `## Handoff Note` at lines 170-185.
  Say explicitly: "This is a small refactor, not a new Fireline product object."

### `docs/execution/16-capability-profiles.md`
Status: stale in both framing and likely numbering.
Recommendation: update, with a numbering decision first.

Concrete deltas:

- Decide whether this file is still slice 16.
  Under the anchor doc, the content here really maps to the future slice 17 concept: portable Tools references with `credential_ref` indirection.
- Rewrite `## Objective` at lines 18-35.
  Replace "CapabilityProfile product object" with "portable tool/capability references, transport refs, and credential refs that products can package into profiles externally."
- Change `## Product Pillar` at lines 37-39.
  Replace "Portable capability profiles" with "Tools references" or "Portable tool references."
- Rewrite `## User Workflow Unlocked` at lines 41-49.
  Make the benefit "the same set of tools can be attached to runs across placements without injecting raw secrets," not "users reuse the same profile object."
- Rewrite `## Why This Slice Exists` at lines 51-63.
  Keep the problem statement about scattered environment concerns, but end with "Fireline should expose tool references and credential indirection, not own the full profile system."
- Delete `### 1. CapabilityProfile object and schema` at lines 67-89.
  Replace it with `### 1. Tool reference schema`, using shapes like `{ name, description, input_schema, transport_ref, credential_ref }`.
- Delete `### 2. Product API surface` at lines 91-104.
  Remove `client.profiles.*`. Replace with a section on how consuming products package these refs into their own profile objects if they want that UX.
- Keep `### 3. Credentials as references, never payloads` at lines 106-123.
  This section is already directionally correct. Update the wording from "profiles store credential references" to "tool references carry `credential_ref` indirection."
- Rewrite `### 4. Compilation into existing substrate surfaces` at lines 125-137.
  Replace "profile compiles into substrate" with "tool refs and instruction/policy refs resolve into topology, MCP injection, and auth resolution at run time."
- Rewrite `### 5. Run integration` at lines 139-147.
  Remove `profileId`. Replace with "launch specs reference tool bundles or tool refs packaged by the consuming product."
- Keep `### 6. First-cut provider neutrality` at lines 149-157, but reword it around transport/provider neutrality of tool refs.
- Rewrite `## Acceptance Criteria` at lines 170-181.
  Remove `CapabilityProfile` object and `client.profiles.*`. Replace with acceptance for tool reference schema, `credential_ref` semantics, and one integration that resolves tools into a real run.
- Rewrite `## Validation` at lines 183-195.
  Replace profile CRUD tests with tool-binding and credential-indirection tests.
- Rewrite `## Handoff Note` at lines 197-212.
  End with "Fireline exposes portable Tools references; consuming products may package them into profiles."

### `docs/execution/17-out-of-band-approvals.md`
Status: stale in both framing and likely numbering.
Recommendation: update, and make it explicitly depend on slice 18.

Concrete deltas:

- Decide whether this file is still slice 17.
  Under the anchor doc, this content really maps to the future slice 16 concept: approvals as a consumer of `wake()`.
- Rewrite `## Objective` at lines 17-31.
  Replace "ApprovalRequest and run wait-state model" with "durable wait records and one approval-service flow that resumes via the Orchestration `wake()` primitive."
- Change `## Product Pillar` at lines 34-36.
  Replace "Reusable conductor extensions" with "Orchestration" or "Pause / wait / resume surface."
- Rewrite `## User Workflow Unlocked` at lines 38-47.
  Make the story "a consuming product can resolve a wait and call `wake()` to continue work" rather than "Fireline exposes a product approval queue."
- Rewrite `## Why This Slice Exists` at lines 50-62.
  Explicitly say this slice is downstream of slice 18, not a substitute for orchestration.
- Rewrite `### 1. ApprovalRequest product object` at lines 66-101.
  Keep the durable record idea, but rename it to "wait record" or "serviceable wait request" and say the human-facing approval object belongs to the consumer product.
- Rewrite `### 2. Run wait-state projection` at lines 103-114.
  Replace "run" language with "session/runtime wait metadata" unless a consuming product chooses to project a run object on top.
- Delete `### 3. Product API surface` at lines 116-133.
  Remove `client.approvals.*` and `client.runs.list({ state: "waiting" })`. Replace with "service hook and resolution contract," likely an HTTP or control-plane callback consumed by products.
- Keep `### 4. One gate path from conductor to durable request` at lines 135-147.
  Reword it to say conductor emits the durable wait record; orchestration owns resume.
- Rewrite `### 5. Resume semantics after service` at lines 149-158.
  This section should explicitly say "approval resolution triggers `wake(session_id)` or equivalent control-plane wake for the owning runtime."
- Rewrite `### 6. One service path` at lines 160-170.
  Make the proof about an external resolver path invoking wake, not about Fireline owning the service UI.
- Rewrite `## Acceptance Criteria` at lines 183-192.
  Replace product API checks with: durable wait record exists, one resolution path writes the decision, and one wake-driven resume path advances the session.
- Rewrite `## Validation` at lines 194-207.
  Replace `client.approvals.*` tests with "resolve wait → wake runtime/session → observe resumed progress."
- Rewrite `## Handoff Note` at lines 209-224.
  Add "This slice is not the orchestration primitive itself; it consumes slice 18."

### `docs/execution/next-steps-proposal.md`
Status: historically useful, but stale as a current planning document.
Recommendation: archive or relabel.

Concrete deltas:

- Add a banner above `## Purpose` at lines 12-19.
  It should say this is a historical decision log from the slice-11 to slice-12 transition and is no longer current roadmap guidance.
- Rewrite `## Current state` at lines 21-35.
  Either remove it or mark every bullet as historical to avoid implying that slice 12 is still the "planned next" step.
- Rewrite `## Immediate follow-on` at lines 85-98.
  This section should either be removed or replaced with a one-line pointer to the active execution README.

### `docs/runtime/heartbeat-and-registration.md`
Status: stale because the doc still describes shipped work as future work.
Recommendation: update.

Concrete deltas:

- Rewrite the status header at lines 3-5.
  Replace "design doc (not yet implemented)" with "reference doc for the shipped push lifecycle, with remaining migration notes."
- Rewrite `## Purpose` at lines 13-23.
  Say 13b landed the push lifecycle for local/provider-backed startup and auth, while polling remains a legacy fallback.
- Rename `## The push-based alternative` at line 66.
  It is no longer an alternative. It should become something like "Current push lifecycle model."
- Keep `### Endpoints`, `### Cadence and timeouts`, `### Authentication`, and `## Status state machine` at lines 70-187.
  These sections are still substantively useful. Update tense from future to present where needed.
- Rewrite `## Backward compatibility with the polling model` at lines 189-226.
  `Slice 13 v2 (proposed)` should become shipped; `Slice 13 v1` should become historical legacy; `v3` and `v4` remain future.
- Rewrite the status lines in `### Slice 13 v2` at lines 202-210.
  Replace "not yet started" with "shipped in 13b" and summarize the actual landed behavior.
- Update `## Runtime-side client sketch` at lines 228-260 and beyond.
  Either mark it explicitly as illustrative pseudocode or replace it with a note pointing at `src/control_plane_client.rs` as the canonical implementation.
- Keep `## Ordering at runtime startup` and `## Invariant preservation vs §4a`.
  These still match the current contract and should stay as reference sections.
- Rewrite `## Migration plan (concrete steps)` at lines 488-514.
  Convert completed steps into a shipped-history list and leave only future items open.
- Rewrite `## When this doc gets updated` at the end.
  The next updates should now be about remote-provider adoption, wake/orchestration integration, and removal of the polling fallback.

## Product docs

### `docs/product/index.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Rewrite `## North Star` at lines 18-27.
  Replace "durable agent fabric" bullets with a short substrate-first statement that points to `Session`, `Orchestration`, `Harness`, `Sandbox`, `Resources`, and `Tools`.
- Rewrite `## Reading Order` at lines 29-63.
  Make `priorities.md` and `managed-agents-mapping.md` the first reading steps. Demote or archive the object-first docs.

### `docs/product/vision.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Rewrite `## Elevator Pitch` at lines 9-22.
  The pitch should say Fireline is substrate beneath orchestration products, not the eventual owner of product-layer runs/workspaces/profiles.
- Rewrite `## The Product Job To Be Done` at lines 24-45.
  Use the six-primitives vocabulary directly and stop centering "capability bundle travels with the run" as a Fireline-owned product object.
- Rewrite `## What This Means` at lines 93-99.
  It should conclude with "Session, Sandbox, and Tools are strong; Orchestration and Resources are the gaps."

### `docs/product/object-model.md`
Status: stale.
Recommendation: archive or replace.

Concrete deltas:

- Replace the entire body from line 11 onward.
  The current sections `Session`, `Workspace`, `Capability Profile`, `Runtime`, and `Agent Run` should not remain the canonical product vocabulary.
- If kept, rewrite it to mirror the six primitives from the anchor doc.
  `Session`, `Orchestration`, `Harness`, `Sandbox`, `Resources`, `Tools` should replace the current object model.
- If archived, replace the body with a banner pointing to `priorities.md` and `managed-agents-mapping.md`.

### `docs/product/runs-and-sessions.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Fix the broken related link at line 12.
  `../execution/09-state-first-session-load.md` does not exist.
- Rewrite `## Purpose` and `## Short Version` at lines 15-37.
  Keep the useful distinction, but say Fireline owns Session as durable evidence and orchestration products own Run if they need it.
- Rewrite `## What A Run Is` at lines 59-78.
  This should become a paragraph about downstream orchestration state, not a Fireline-owned product object.
- Rewrite `## What Fireline Already Has` at lines 116-134.
  Replace "missing `client.sessions` / `client.runs` product surface" with "missing canonical session read surface and orchestration integration seams."
- Delete `## Product Responsibilities` and its subsections at lines 136-182.
  Remove all `client.runs` / `client.sessions` API shaping.
- Delete or heavily rewrite `## Suggested Product Shapes` at lines 184-222.
  Those object types are no longer Fireline's contract.
- Rewrite `## Waiting And Approvals` at lines 241-252.
  Tie it to Orchestration and `wake()`, not Fireline-owned approval objects.
- Rewrite `## First-Cut Recommendation` and `## What Future Slices Should Prove` later in the doc.
  Align them to slice 14 as Session read surface plus slice 18 as Orchestration.

### `docs/product/workspaces.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Rewrite `## Purpose` at lines 13-24.
  Replace "first-class workspace object" with "Resources primitive for launch inputs."
- Rewrite `## What A Workspace Is` through `## What A Workspace Is Not` at lines 26-55.
  Rename this vocabulary to `Resources` or explicitly mark workspace as a downstream packaging choice, not a Fireline-owned object.
- Rewrite `## Product Questions A Workspace Should Answer` and the four mode sections at lines 76-139.
  Recast them as supported `source_ref` kinds and materialization strategies, not a user-facing workspace model.
- Delete `## Suggested Product Surface` and `## Strawman Workspace Shape` at lines 189-221.
  Replace with a short `ResourceRef` example and a note about `ResourceMounter`.
- Rewrite `## First-Cut Recommendation` at lines 253-272.
  Say "demote to a small refactor" and point to the Resources primitive from the anchor doc.

### `docs/product/capability-profiles.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Rewrite `## Purpose` and `## What A Capability Profile Is` at lines 13-42.
  Make profiles explicitly a downstream packaging concept; Fireline should expose tool references, policy refs, and credential refs.
- Rewrite `## Core Boundaries` at lines 64-95.
  Keep the separation from runtime/session/resources, but make `Tools` the primitive and "profile" the optional packaging layer above Fireline.
- Rewrite `## What Should Belong In A Profile` at lines 97-164.
  Move from "profile contents" to "Fireline-supported tool reference fields," especially `transport_ref` and `credential_ref`.
- Delete `## Product Shape` and `## Strawman Profile Shape` at lines 178-225.
  Replace with a tool-reference shape derived from the anchor doc.
- Keep and sharpen `## agent.pw As The Credential Layer` at lines 241-257.
  This section is still good, but it should say `credential_ref` rather than "profile stores refs."
- Rewrite `## First-Cut Recommendation` and `## Questions The Next Slice Should Answer` later in the doc.
  Align them to the future slice 17 framing from the anchor doc.

### `docs/product/out-of-band-approvals.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Rewrite `## Purpose` and `## Core Product Behavior` at lines 13-67.
  Replace product-layer wording with orchestration wording: durable wait record plus external resolution plus `wake()`.
- Delete `## Product Objects` and its subsections at lines 69-94.
  Replace with "durable wait record" and "wake trigger" language.
- Rewrite `## Strawman Approval Shape` at lines 96-122.
  Keep only fields needed for durable wait/service semantics; remove assumptions that this is Fireline's user-facing approval object.
- Rewrite `## Product Surface` at lines 203-221.
  Remove `client.approvals.*`. Replace with "resolution hook surface used by consuming products."
- Rewrite `## What Actually Resumes The Run` at lines 224-240.
  State explicitly that the answer is slice 18's `wake()` primitive.
- Rewrite `## First-Cut Recommendation` and `## Questions The Next Slice Should Answer` later in the doc.
  Tie them to the orchestration dependency rather than independent Fireline approval APIs.

### `docs/product/product-api-surfaces.md`
Status: stale.
Recommendation: archive or replace.

Concrete deltas:

- Replace the body from `## The Core Separation` at line 33 onward.
  The current proposal for `client.sessions`, `client.workspaces`, `client.profiles`, `client.runs`, and `client.approvals` directly contradicts the substrate-first direction.
- If kept, rewrite it as "substrate integration surfaces."
  It should document how products consume `client.host`, `client.acp`, `client.state`, `client.topology`, plus future Session read helpers and Orchestration hooks.
- Remove all namespace sections at lines 135-271 and the strawman product client at lines 410-451.
  Those are the most misleading parts of the current product folder.

### `docs/product/user-surfaces.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Rewrite `## What End Users Should Actually Do` at lines 10-26.
  The current flow assumes direct Fireline-owned workspace/profile/run UX.
- Rewrite the example flows at lines 28-94.
  Keep the workflow value, but make the actor a product built on Fireline rather than Fireline itself.
- Rewrite `## A More Product-Like API Direction` at lines 209 onward.
  Remove direct `client.workspaces`, `client.profiles`, and `client.runs` examples.

### `docs/product/ecosystem-story.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Rewrite `### Managed-agent platforms` at lines 60-75.
  Make this section explicitly map Fireline to the six primitives instead of vague managed-agent parity.
- Rewrite the ownership sentence in `## agent.pw Integration Story` around lines 126-148.
  Remove "Fireline owns sessions, runs, approvals, and runtime placement." Replace with substrate ownership language consistent with `priorities.md`.
- Rewrite `## Product Positioning Implication` at lines 205 onward.
  End with "Fireline is the substrate for Session/Sandbox/Tools, with Orchestration and Resources as the remaining gaps."

### `docs/product/roadmap-alignment.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Rewrite `## Product Pillars` at lines 26-37.
  Replace the current five pillars with the six managed-agent primitives or a direct mapping onto them.
- Rewrite `### 2. Session is durable but not yet a real product object` at lines 131-140.
  It should say "Session is strong; the gap is canonical read surface."
- Rewrite `### 3. Workspace and capability profile do not exist yet` at lines 141-149.
  Replace with "Resources is missing; Tools references need a sharper contract."
- Add a new subsection under `## What The Current Slice Set Still Does Not Deliver`.
  It should name `Orchestration / wake` as the largest missing primitive.
- Rewrite `## Slice Selection Rule` at lines 175-249.
  The "Product object check" should become a "managed-agent primitive check."
- Rewrite `## Recommended Next Slice Sequence` at lines 251 onward.
  `13b` should be renamed to push lifecycle/auth in the historical sequence, `15` should be demoted to a refactor, `16` should depend on new `18`, and `17` should be reframed as tool references with `credential_ref`.

### `docs/product/backlog.md`
Status: stale.
Recommendation: update.

Concrete deltas:

- Rewrite the backlog rows at lines 24-38.
  `13b` should no longer be "Docker provider + mixed topology." `14` should become session read surface. `15` should become a Resources refactor, not a slice. `16` and `17` should match the anchor doc's reframed semantics. `18` should become "Orchestration and wake," not ACP augmentation.
- Add a note below the table.
  It should explain the numbering mismatch caused by demoting 15 and inserting 18.

### `docs/product/priorities.md`
Status: mostly aligned, but still needs cleanup.
Recommendation: update lightly.

Concrete deltas:

- Rewrite the `Related` block at lines 3-13.
  It should point to `managed-agents-mapping.md` and stop depending on stale object-first docs for support.
- In `## Fireline Surface Area To Prioritize`, update headings 3, 5, and 6 at lines 179-240.
  These are already close; they should explicitly use `Orchestration`, `Tools`, and `Resources` vocabulary from the anchor doc.
- In `## Recommended Slice Ordering` at lines 264-339, make the numbering decision explicit.
  It already gestures at the reframe, but it should call out slice 18 as new and say whether filenames will be renumbered or only reframed.

## Bottom line

The anchor doc changes the docs problem from "tune a few product nouns" to
"stop treating Fireline as the owner of product objects."

The concrete cleanup sequence is:

1. rewrite slice 14 as Session read surface
2. demote slice 15 into Resources refactor
3. fix the 16/17 numbering and framing mismatch
4. add slice 18 for Orchestration / `wake`
5. archive or replace the product docs that still assume
   `client.runs/workspaces/profiles/approvals`

That is the shortest path to getting the docs back in sync with
`priorities.md` and `managed-agents-mapping.md`.
