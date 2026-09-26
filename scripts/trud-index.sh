#!/usr/bin/env bash
#
# scripts/trud-index.sh <releases.json> <listing.json> [<date>…]
#
# Writes to stdout the release index a build uses: <releases.json> (normally the tag's
# data/releases.json) with a release row added for each named date — every date in
# <listing.json> when none is named — that the index doesn't already have. <listing.json> is
# `ods trud list --all --format json`'s output: each item's `date`, `sha256` and `size_bytes`.
#
# A new row is:
#   { "trud_release_date": "<date>", "trud_release_sha256": "<sha256>",
#     "trud_release_filesize_bytes": <size_bytes>, "datasets": [] }
#
# For a date the index already has, the row's SHA-256 and size are checked against the listing:
# any difference exits 1, naming both. A named date the listing doesn't have also exits 1. The
# index is append-only, so this never rewrites or drops a row; it only adds. Rows in the output
# stay in date order (newest first), and the output validates as a releases.v1 index.
#
set -euo pipefail

usage() {
  echo "usage: scripts/trud-index.sh <releases.json> <listing.json> [<date>...]" >&2
  exit 2
}

[[ $# -ge 2 ]] || usage
RELEASES="$1"
LISTING="$2"
shift 2

[[ -f "$RELEASES" ]] || { echo "✖ $RELEASES isn't a file" >&2; exit 2; }
[[ -f "$LISTING" ]] || { echo "✖ $LISTING isn't a file" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "✖ jq is needed and isn't installed" >&2; exit 2; }

# a hex hash, abbreviated for a one-line diagnostic
short() {
  local h="$1"
  echo "${h:0:8}…"
}

DATES=("$@")
if [[ ${#DATES[@]} -eq 0 ]]; then
  mapfile -t DATES < <(jq -r '.[].date' "$LISTING")
fi

declare -a NEW_ROWS=()

for date in "${DATES[@]}"; do
  listing_row="$(jq -c --arg d "$date" '[.[] | select(.date == $d)][0]' "$LISTING")"
  if [[ "$listing_row" == "null" ]]; then
    echo "✖ $date is not in TRUD's listing" >&2
    exit 1
  fi
  listing_sha="$(jq -r '.sha256' <<<"$listing_row")"
  listing_size="$(jq -r '.size_bytes' <<<"$listing_row")"

  index_row="$(jq -c --arg d "$date" '[.releases[] | select(.trud_release_date == $d)][0]' "$RELEASES")"
  if [[ "$index_row" != "null" ]]; then
    index_sha="$(jq -r '.trud_release_sha256' <<<"$index_row")"
    index_size="$(jq -r '.trud_release_filesize_bytes' <<<"$index_row")"
    if [[ "$index_sha" != "$listing_sha" || "$index_size" != "$listing_size" ]]; then
      echo "✖ TRUD's listing disagrees with data/releases.json for $date" >&2
      echo "  the index   $(short "$index_sha")  $index_size bytes" >&2
      echo "  TRUD says   $(short "$listing_sha")  $listing_size bytes" >&2
      echo "  The index is append-only. If TRUD changed its account of a release, that's news to report." >&2
      exit 1
    fi
    continue
  fi

  NEW_ROWS+=("$(jq -n --arg d "$date" --arg sha "$listing_sha" --argjson size "$listing_size" \
    '{trud_release_date: $d, trud_release_sha256: $sha, trud_release_filesize_bytes: $size, datasets: []}')")
done

new_rows_json="$(printf '%s\n' ${NEW_ROWS[@]+"${NEW_ROWS[@]}"} | jq -s '.')"

jq --argjson new "$new_rows_json" \
  '.releases += $new
   | .releases |= (unique_by(.trud_release_date) | sort_by(.trud_release_date) | reverse)' \
  "$RELEASES"
