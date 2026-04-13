#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Phase 8 extended rg gate for canonical-ids cleanup.
#
# This script is intentionally built from the actual migration diffs rather than
# only the short baseline in docs/proposals/acp-canonical-identifiers-execution.md
# / bead mono-vkpp.10.
#
# Diff-mined sources:
# - 3a75a06  Phase 0 + 1 shared type layer
# - 104285b  Phase 1.5 agent-layer fields typed as ACP ids
# - 074b14e  Phase 2 approval gate switches from synthetic ids to canonical JSON-RPC ids
# - f9a4f74  Phase 3 canonical state_projector rekeying + child_session_edge deletion
# - 714cc84  Phase 3.5 chunk/session-update typed payload migration
# - 429475e  Phase 4 W3C trace context replaces legacy fireline trace lineage fields
# - 2036c9e  Phase 5 deletes ActiveTurnIndex
# - 1ba64c1  Phase 6 packages/state promptRequests migration + compat aliases
#
# Notes for the eventual Phase 8 cleanup:
# - The hard-fail list below is the vocabulary that should be gone once
#   migration scaffolding is deleted.
# - Canonical survivors like `session_v2` and `chunk_v2` are tracked as
#   review-only inventory, not hard failures. They appeared in migration diffs,
#   but they are the steady-state names today.
# - Phase 7 plane-separation cleanup is represented as a scoped hard-fail sweep
#   over agent-plane files only. A repo-wide rg on runtime/host ids would be
#   too noisy because those identifiers remain valid in infrastructure-plane code.
# - Current main is expected to fail before Phase 7/8 lands. Typical pre-cleanup
#   stragglers today live in:
#   - crates/fireline-harness/src/state_projector.rs
#   - crates/fireline-session/src/session_index.rs
#   - packages/state/src/schema.ts
#   - packages/state/src/collection.ts
#   - packages/state/src/collections/{active-turns,queued-turns,session-turns,turn-chunks}.ts
#   - packages/state/test/fixtures/rust-state-producer.ndjson

SEARCH_ROOTS=(crates packages)
RG_COMMON_ARGS=(
  --line-number
  --color=never
  --glob '!**/dist/**'
  --glob '!**/node_modules/**'
)

tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

cat >"$tmpdir/hard-fail.patterns" <<'EOF'
PromptTurn(Row|State|EventRow)\b
prompt_turn
promptTurnId
prompt_turn_id
parentPromptTurnId
parent_prompt_turn_id
logicalConnectionId
logical_connection_id
traceId
trace_id
chunkId
chunk_id
chunkSeq
chunk_seq
child_session_edge
ActiveTurnIndex
ActiveTurnRecord
approval_request_id
prompt_identity
fireline_trace_id
LegacyPromptTurn\w*
LegacySession\w*
LegacyChunk\w*
legacyPromptTurn\w*
legacySession\w*
legacyChunk\w*
legacy_prompt_turn\w*
legacy_session\w*
legacy_chunk\w*
createSessionTurnsCollection
createTurnChunksCollection
SessionTurnsOptions
TurnChunksOptions
promptTurns\b
type:\s*['"]prompt_turn['"]
type:\s*['"]session['"]
type:\s*['"]chunk['"]
Some\(["']prompt_turn["']\)
Some\(["']session["']\)
Some\(["']chunk["']\)
state_change\(["']prompt_turn["']
state_change\(["']session["']
state_change\(["']chunk["']
EOF

cat >"$tmpdir/phase7-scoped.patterns" <<'EOF'
runtimeKey
runtimeId
nodeId
hostKey
hostId
providerInstanceId
EOF

cat >"$tmpdir/review-only.patterns" <<'EOF'
session_v2
chunk_v2
EOF

PHASE7_SCOPED_PATHS=(
  crates/fireline-harness/src/state_projector.rs
  crates/fireline-session/src/lib.rs
  crates/fireline-session/src/session_index.rs
  packages/state/src
)

fail=0

echo "== Phase 8 extended rg gate =="
echo

echo "-- Hard-fail: legacy canonical-ids vocabulary slated for deletion --"
if rg "${RG_COMMON_ARGS[@]}" -f "$tmpdir/hard-fail.patterns" "${SEARCH_ROOTS[@]}"; then
  fail=1
else
  echo "No hard-fail legacy/shim names found."
fi
echo

echo "-- Hard-fail: agent-plane files still exposing infrastructure ids (Phase 7 scope) --"
if rg "${RG_COMMON_ARGS[@]}" -f "$tmpdir/phase7-scoped.patterns" "${PHASE7_SCOPED_PATHS[@]}"; then
  fail=1
else
  echo "No scoped infrastructure-plane leaks found in agent-plane files."
fi
echo

echo "-- Review-only: canonical survivor inventory from migration diffs --"
if rg "${RG_COMMON_ARGS[@]}" -f "$tmpdir/review-only.patterns" "${SEARCH_ROOTS[@]}"; then
  echo
  echo "Review-only hits above are inventory, not gate failures."
else
  echo "No review-only survivor hits found."
fi
echo

if [[ "$fail" -ne 0 ]]; then
  cat <<'EOF'
FAIL: the extended Phase 8 rg gate found names that should disappear once the
canonical-ids migration scaffolding is deleted.

Current main is expected to fail before Phase 7/8 lands. Focus the cleanup on:
- legacy prompt/session/chunk readers and entity types
- TS compat aliases around promptRequests/requestChunks
- residual prompt_turn / traceId / logicalConnectionId vocabulary
- agent-plane rows still carrying infrastructure identifiers
EOF
  exit 1
fi

echo "PASS: no extended Phase 8 cleanup leftovers were found."
