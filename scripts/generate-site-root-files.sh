#!/usr/bin/env bash
# Lists the unhashed files site/dist ships at the root (everything except index.html and
# _astro/, which the worker already routes by other means) and writes worker/src/site-root-files.json
# — the worker imports this so its route list can never drift from what a build actually produced.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$REPO_ROOT/site/dist"
OUT="$REPO_ROOT/worker/src/site-root-files.json"

if [ ! -d "$DIST" ]; then
  echo "missing $DIST — run 'astro build' first" >&2
  exit 1
fi

find "$DIST" -maxdepth 1 -type f ! -name 'index.html' -exec basename {} \; \
  | sort \
  | jq -R -s 'split("\n") | map(select(length > 0)) | map("/" + .)' \
  > "$OUT"

echo "wrote $OUT"
