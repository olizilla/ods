#!/usr/bin/env bash
#
# scripts/ci-verify-sources.sh <releases.json> <index.json> <plan.json>
#
# build-dataset.yml's "Verify source: nhs-ods-xml" step. For every {date, bundle_digest} in
# <plan.json>: pulls that source bundle from nhs-ods-xml by digest into work/<date>/trud/, checks
# NHS's signature against the key pinned in <releases.json> (the tag's data/releases.json —
# never the run's own index.json, which only supplies the date's SHA-256 and size), then checks
# the zip's own SHA-256 and size against <index.json>'s row for that date. Adds a row to the job
# summary's Source table for every date, in order.
#
# Any ✖ — a failed signature, or a mismatch against the index — stops here and exits 1, before
# scripts/ci-build-datasets.sh runs for any date: this script finishes every date it can check
# before scripts/ci-build-datasets.sh builds anything.
#
# Needs oras and jq, and a working `oras login` already done by the caller: nhs-ods-xml is
# private. TRUD_LISTING_STATUS ("checked", or "not checked: <reason>") is shown in the Source
# table, the same for every date.
#
set -euo pipefail

usage() {
  echo "usage: scripts/ci-verify-sources.sh <releases.json> <index.json> <plan.json>" >&2
  exit 2
}

[[ $# -eq 3 ]] || usage
RELEASES="$1"
INDEX="$2"
PLAN="$3"
[[ -f "$RELEASES" ]] || { echo "✖ $RELEASES isn't a file" >&2; exit 2; }
[[ -f "$INDEX" ]] || { echo "✖ $INDEX isn't a file" >&2; exit 2; }
[[ -f "$PLAN" ]] || { echo "✖ $PLAN isn't a file" >&2; exit 2; }
command -v oras >/dev/null 2>&1 || { echo "✖ oras is needed and isn't installed" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "✖ jq is needed and isn't installed" >&2; exit 2; }

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUMMARY="${GITHUB_STEP_SUMMARY:-/dev/null}"
TRUD_LISTING_STATUS="${TRUD_LISTING_STATUS:-not checked: TRUD_LISTING_STATUS is unset}"

mkdir -p work

header_written=false
summary_header() {
  $header_written && return 0
  {
    echo "### Source"
    echo "| date | bundle | zip sha256 | NHS signature | SHA-1 | index row | TRUD listing (this run) |"
    echo "| :--- | :--- | :--- | :--- | :--- | :--- | :--- |"
  } >>"$SUMMARY"
  header_written=true
}

# sha256:<hex>, abbreviated for a table cell, prefix kept
short_ref() {
  local h="${1#sha256:}"
  echo "sha256:${h:0:8}…${h: -4}"
}

# a bare hex hash (zip SHA-256, never prefixed in this codebase), abbreviated for a table cell
short_hash() {
  local h="$1"
  echo "${h:0:8}…${h: -4}"
}

# the Source table's last column: ✓ when this run's TRUD listing was checked, else the reason
listing_cell() {
  [[ "$TRUD_LISTING_STATUS" == "checked" ]] && echo "✓" || echo "* $TRUD_LISTING_STATUS"
}

while IFS= read -r entry; do
  date="$(jq -r '.date' <<<"$entry")"
  bundle_digest="$(jq -r '.bundle_digest' <<<"$entry")"

  trud_dir="work/$date/trud"
  mkdir -p "$trud_dir"

  echo "::group::verify $date"
  echo "* pulling nhs-ods-xml@$bundle_digest"
  oras pull "ghcr.io/olizilla/nhs-ods-xml@$bundle_digest" -o "$trud_dir" >/dev/null

  if ! verified="$("$REPO_ROOT/scripts/verify-trud-bundle.sh" "$trud_dir" "$RELEASES" 2>&1)"; then
    echo "$verified" >&2
    summary_header
    echo "| $date | \`$(short_ref "$bundle_digest")\` | – | ✖ | ✖ | – | $(listing_cell) |" >>"$SUMMARY"
    echo "::endgroup::"
    exit 1
  fi
  echo "$verified"

  zip_path="$(find "$trud_dir" -maxdepth 1 -name '*.zip' | head -n 1)"
  zip_sha="$(sed -n 's/.*SHA-256 \([0-9A-Fa-f]*\)$/\1/p' <<<"$verified")"
  zip_sha_upper="$(tr '[:lower:]' '[:upper:]' <<<"$zip_sha")"
  zip_size="$(wc -c <"$zip_path" | tr -d ' ')"
  fingerprint_line="$(sed -n 's/^✓ [^ ]* signed by \([^ ]*\) on \([^;]*\);.*/\1, signature dated \2/p' <<<"$verified")"

  index_row="$(jq -c --arg d "$date" '[.releases[] | select(.trud_release_date == $d)][0]' "$INDEX")"
  if [[ "$index_row" == "null" ]]; then
    echo "✖ $date has no row in $INDEX" >&2
    summary_header
    echo "| $date | \`$(short_ref "$bundle_digest")\` | \`$(short_hash "$zip_sha")\` | $fingerprint_line | ✓ | ✖ | $(listing_cell) |" >>"$SUMMARY"
    echo "::endgroup::"
    exit 1
  fi
  index_sha="$(jq -r '.trud_release_sha256' <<<"$index_row")"
  index_size="$(jq -r '.trud_release_filesize_bytes' <<<"$index_row")"

  if [[ "$zip_sha_upper" != "$index_sha" || "$zip_size" != "$index_size" ]]; then
    summary_header
    echo "| $date | \`$(short_ref "$bundle_digest")\` | \`$(short_hash "$zip_sha")\` | $fingerprint_line | ✓ | ✖ | $(listing_cell) |" >>"$SUMMARY"
    echo "✖ $date  the zip doesn't match $INDEX's row" >&2
    echo "  the zip    $zip_sha_upper  $zip_size bytes" >&2
    echo "  the index  $index_sha  $index_size bytes" >&2
    echo "::endgroup::"
    exit 1
  fi

  summary_header
  echo "| $date | \`$(short_ref "$bundle_digest")\` | \`$(short_hash "$zip_sha")\` | $fingerprint_line | ✓ | ✓ | $(listing_cell) |" >>"$SUMMARY"
  echo "✓ $date  verified"
  echo "::endgroup::"
done < <(jq -c '.[]' "$PLAN")
