#!/usr/bin/env bash
#
# scripts/compare-digests.sh <a> <b>
#
# Compares two digest lists, one "<date> <manifest digest>" per line, as the build-dataset.yml
# build and witness jobs each record for every date they build. Exits 0 when every date names the
# same digest in both. On any difference — a mismatched digest, or a date only one side has —
# prints a table of the dates that differ, with both digests, and exits 1.
#
set -euo pipefail

usage() {
  echo "usage: scripts/compare-digests.sh <a> <b>" >&2
  exit 2
}

[[ $# -eq 2 ]] || usage
A="$1"
B="$2"
[[ -f "$A" ]] || { echo "✖ $A isn't a file" >&2; exit 2; }
[[ -f "$B" ]] || { echo "✖ $B isn't a file" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

sort -k1,1 "$A" >"$WORK/a.sorted"
sort -k1,1 "$B" >"$WORK/b.sorted"

joined="$(join -j 1 -a 1 -a 2 -e MISSING -o 0,1.2,2.2 "$WORK/a.sorted" "$WORK/b.sorted")"
mismatches="$(awk '$2 != $3' <<<"$joined")"

if [[ -n "$mismatches" ]]; then
  count="$(wc -l <<<"$mismatches" | tr -d ' ')"
  echo "✖ the builders disagree on $count date(s)" >&2
  printf '%-12s  %-74s  %s\n' "date" "$(basename "$A")" "$(basename "$B")" >&2
  while read -r date digest_a digest_b; do
    printf '%-12s  %-74s  %s\n' "$date" "$digest_a" "$digest_b" >&2
  done <<<"$mismatches"
  exit 1
fi

count="$(wc -l <"$WORK/a.sorted" | tr -d ' ')"
echo "✓ $count date(s) match"
