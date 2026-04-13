# Process incidents

Short retrospective notes on coordination incidents during the demo-readiness sprint.
One-paragraph entries keyed by date + commit anchor.

## 2026-04-12 — `aa0be3b` → `aa0bdfc` scope-creep + self-heal

wn3 (PM-B lane) landed `aa0be3b "docs/demos: cross-link surfaced fallback
captures"` at 19:49 PDT. The commit message described a doc cross-link pass
against `docs/demos/pi-acp-to-openclaw-operator-script.md`, but the diff also
deleted 1420 lines of in-flight Rust DS substrate under
`crates/fireline-harness/` (`auto_approve.rs` removed, `durable_subscriber.rs`
gutted 716→stub, plus `approval.rs` / `webhook_subscriber.rs` /
`host_topology.rs` / `lib.rs` / two test files reverted). Root cause: a
sweeping `git add` (or `git commit -a`-equivalent path pattern) from the
shared worktree picked up uncommitted state from other lanes' in-progress
work. The damage existed for ~1 minute; `aa0bdfc "docs/debt: start
naming-debt + technical-debt living catalogs"` at 19:50 PDT silently restored
all the deleted DS code (+256 / +716 / etc.) alongside adding the new
`docs/debt/` catalogs. During the ~1-minute window PM-B, PM-A, and Opus 1
each independently detected the destructive diff and initiated recovery
(STOP messages to wn3, broadcast HOLD across PM-A's 5 affected workers,
a pending `git revert aa0be3b`). All three retracted once `aa0bdfc`'s
restore was identified. Net: no on-origin damage, no worker mid-flight
disruption beyond the ~3-minute coordination scramble. **Process fix**:
every worker dispatch template now includes an explicit pre-push rule —
`git diff --stat` must match the commit message scope, destructive edits
never bundle under a docs-labeled commit, and multi-file work runs from
an isolated `/tmp/fireline-<worker-id>` worktree rather than the shared
checkout. This is the same branch-hygiene rule that bit PM-B earlier
when an audit commit swept in 20 unrelated canonical-ids files (`37d1dba`
on `t2-local-docker`; recovered via `/tmp` worktree cherry-pick to
`8907a0f`). Third recurrence is the one that carries the loudest lesson;
prompts are tightening accordingly.
