#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Phase 8 rg gate for DurableSubscriber cleanup.
#
# This gate is intentionally curated from the actual rollout diffs rather than
# from the proposal prose alone.
#
# Diff-mined sources so far:
# - 8696cc2  mono-axr.1 Phase 1 Rust trait surface
# - 4c5b207  mono-axr.9 Phase 2 approval gate ports onto DurableSubscriber
# - 658d6b3  mono-axr.4 Phase 5 peer-routing subscriber
#
# mono-axr.6 (TypeScript middleware surface) is still in progress. Once it
# lands, extend the hard-fail list with any temporary compat names introduced by
# that phase before closing mono-axr.8.

RG_COMMON_ARGS=(
  --line-number
  --color=never
  --glob '!**/dist/**'
  --glob '!**/node_modules/**'
)

SEARCH_PATHS=(
  crates/fireline-harness/src/durable_subscriber.rs
  crates/fireline-harness/src/approval.rs
  crates/fireline-harness/src/peer_routing.rs
  packages/client/src/middleware.ts
  packages/client/src/types.ts
)

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

cat >"$tmpdir/hard-fail.patterns" <<'EOF'
Durable subscriber substrate scaffolding
Phase 1 intentionally lands only the Rust trait and registration surface
The driver remains inert until Phase 2 ports the approval gate onto it
Registration mode for the inert Phase 1 driver
Phase 2 ports approval timeout behavior onto this
Phase 1 registration-only driver
That keeps the substrate inert until the first real consumer ports onto it in Phase 2
\bdurableSubscriberCompat\b
\blegacyDurableSubscriber\b
\blegacy_completion_key\b
\bsyntheticCompletionKey\b
\bcompletionKeyAlias\b
\bpassiveSubscriberCompat\b
\bactiveSubscriberCompat\b
EOF

cat >"$tmpdir/review-only.patterns" <<'EOF'
\bDurableSubscriber\b
\bPassiveSubscriber\b
\bActiveSubscriber\b
\bCompletionKey\b
\bPeerRoutingSubscriber\b
\bdurableSubscriber\(
EOF

fail=0

echo "== DurableSubscriber Phase 8 rg gate =="
echo

echo "-- Hard-fail: rollout scaffolding and compat aliases that should disappear --"
if rg "${RG_COMMON_ARGS[@]}" -f "$tmpdir/hard-fail.patterns" "${SEARCH_PATHS[@]}"; then
  fail=1
else
  echo "No rollout scaffolding or compat aliases found."
fi
echo

echo "-- Review-only: steady-state canonical surface inventory --"
if rg "${RG_COMMON_ARGS[@]}" -f "$tmpdir/review-only.patterns" "${SEARCH_PATHS[@]}"; then
  echo
  echo "Review-only hits above are inventory, not gate failures."
else
  echo "No review-only canonical surface hits found."
fi
echo

if [[ "$fail" -ne 0 ]]; then
  cat <<'EOF'
FAIL: the DurableSubscriber Phase 8 rg gate found rollout scaffolding or
compatibility names that should be removed before mono-axr.8 closes.

Focus the cleanup on:
- stale Phase 1 / Phase 2 scaffolding commentary
- temporary compat aliases around the TS middleware surface
- any synthetic completion-key helpers that bypass canonical ACP ids
EOF
  exit 1
fi

echo "PASS: no Phase 8 DurableSubscriber cleanup leftovers were found."
