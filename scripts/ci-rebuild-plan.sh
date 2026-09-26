#!/usr/bin/env bash
#
# scripts/ci-rebuild-plan.sh <releases.json> <plan-out> <index-out> [<date>...]
#
# rebuild-datasets.yml's `plan` job: resolves the dates to rebuild (every date named, or every
# tag in nhs-ods-xml when none is), each one's archive digest (`oras resolve`), and this run's
# index. Writes <plan-out> ({date, bundle_digest} per date) and <index-out>.
#
# Tries `ods trud list --all --format json` (needs ODS_BIN and TRUD_API_KEY; ODS_BIN defaults to
# ./target/release/ods). If it answers, <index-out> is scripts/trud-index.sh's output for exactly
# these dates — "TRUD listing: checked". If it doesn't, <index-out> is <releases.json> unchanged,
# "TRUD listing: not checked: TRUD unreachable" — and any date without a row there fails the
# whole run before anything is planned, each one named.
#
# The last line of stdout is always "TRUD listing: <status>", for the caller to read.
#
set -euo pipefail

usage() {
  echo "usage: scripts/ci-rebuild-plan.sh <releases.json> <plan-out> <index-out> [<date>...]" >&2
  exit 2
}

[[ $# -ge 3 ]] || usage
RELEASES="$1"
PLAN_OUT="$2"
INDEX_OUT="$3"
shift 3
DATES=("$@")

[[ -f "$RELEASES" ]] || { echo "✖ $RELEASES isn't a file" >&2; exit 2; }
command -v oras >/dev/null 2>&1 || { echo "✖ oras is needed and isn't installed" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "✖ jq is needed and isn't installed" >&2; exit 2; }

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ODS="${ODS_BIN:-./target/release/ods}"

if [[ ${#DATES[@]} -eq 0 ]]; then
  mapfile -t DATES < <(oras repo tags ghcr.io/olizilla/nhs-ods-xml | grep -E '^[0-9]{4}-[0-9]{2}-[0-9]{2}$' | sort)
fi
[[ ${#DATES[@]} -gt 0 ]] || { echo "✖ no dates to rebuild: nhs-ods-xml has no date tags" >&2; exit 1; }

: >"$PLAN_OUT.tmp"
for date in "${DATES[@]}"; do
  digest="$(oras resolve "ghcr.io/olizilla/nhs-ods-xml:$date")"
  jq -n --arg d "$date" --arg dig "$digest" '{date: $d, bundle_digest: $dig}' >>"$PLAN_OUT.tmp"
done
jq -s '.' "$PLAN_OUT.tmp" >"$PLAN_OUT"
rm -f "$PLAN_OUT.tmp"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

status="not checked: TRUD unreachable"
listing_ok=false
if [[ -x "$ODS" ]] && listing="$("$ODS" trud list --all --format json 2>"$WORK/listing.err")"; then
  listing_ok=true
fi

if [[ "$listing_ok" == true ]]; then
  echo "$listing" >"$WORK/listing.json"
  "$REPO_ROOT/scripts/trud-index.sh" "$RELEASES" "$WORK/listing.json" "${DATES[@]}" >"$INDEX_OUT"
  status="checked"
else
  if [[ -s "$WORK/listing.err" ]]; then
    sed 's/^/  /' "$WORK/listing.err" >&2
  fi
  cp "$RELEASES" "$INDEX_OUT"
  missing=false
  for date in "${DATES[@]}"; do
    if ! jq -e --arg d "$date" '.releases[] | select(.trud_release_date == $d)' "$INDEX_OUT" >/dev/null; then
      echo "✖ $date has no row in $RELEASES and TRUD is unreachable" >&2
      missing=true
    fi
  done
  if [[ "$missing" == true ]]; then
    exit 1
  fi
fi

echo "TRUD listing: $status"
