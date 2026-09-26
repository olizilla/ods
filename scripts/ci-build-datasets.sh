#!/usr/bin/env bash
#
# scripts/ci-build-datasets.sh <index.json> <plan.json>
#
# build-dataset.yml's "Build every date" step, run only after scripts/ci-verify-sources.sh has
# verified every date in <plan.json> (work/<date>/trud/ already holds a verified source bundle
# for each). For every {date} in <plan.json>: `ods make -i <zip> --index <index.json> -o
# work/<date>/release`, then `ods make oci` on it, then records "<date> <manifest digest>" in
# digests.txt.
#
# Needs jq and ODS_BIN (default ./target/release/ods) already built.
#
set -euo pipefail

usage() {
  echo "usage: scripts/ci-build-datasets.sh <index.json> <plan.json>" >&2
  exit 2
}

[[ $# -eq 2 ]] || usage
INDEX="$1"
PLAN="$2"
[[ -f "$INDEX" ]] || { echo "✖ $INDEX isn't a file" >&2; exit 2; }
[[ -f "$PLAN" ]] || { echo "✖ $PLAN isn't a file" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "✖ jq is needed and isn't installed" >&2; exit 2; }

ODS="${ODS_BIN:-./target/release/ods}"
[[ -x "$ODS" ]] || { echo "✖ $ODS isn't built. Run: cargo build --release --locked" >&2; exit 2; }

: >digests.txt

while IFS= read -r entry; do
  date="$(jq -r '.date' <<<"$entry")"
  work_dir="work/$date"
  trud_dir="$work_dir/trud"
  zip_path="$(find "$trud_dir" -maxdepth 1 -name '*.zip' | head -n 1)"
  [[ -n "$zip_path" ]] || { echo "✖ $date  no zip in $trud_dir — run scripts/ci-verify-sources.sh first" >&2; exit 1; }

  echo "::group::build $date"
  release_dir="$work_dir/release"
  "$ODS" make -i "$zip_path" --index "$INDEX" -o "$release_dir"
  "$ODS" make oci -i "$release_dir"

  manifest_path="$(find "$release_dir/oci/blobs/sha256" -type f ! -type l | head -n 1)"
  manifest_digest="sha256:$(basename "$manifest_path")"
  echo "$date $manifest_digest" >>digests.txt
  echo "✓ $date  built $manifest_digest"
  echo "::endgroup::"
done < <(jq -c '.[]' "$PLAN")
