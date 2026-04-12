#!/usr/bin/env bash
set -euo pipefail

curl -fsS "$1" >/dev/null

if (($# > 1)); then
  curl -fsS "$2" >/dev/null
fi
