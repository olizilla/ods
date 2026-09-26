#!/usr/bin/env bash
#
# scripts/ci-collect-digests.sh <tar-dir> <out>
#
# Extracts every "<date>.tar" in <tar-dir> (each holding "<date>/release/…", the shape
# .github/actions/build-release/action.yml tars a date's release directory as) and writes
# "<date> <manifest digest>" to <out>, one line per date. build-dataset.yml's `gate` job uses
# this to build the two digest lists scripts/compare-digests.sh then compares, one call per
# runner, after downloading every date's per-date artifact for that runner
# (`release-dir-<os>-<date>`, merged into one directory).
#
set -euo pipefail

usage() {
  echo "usage: scripts/ci-collect-digests.sh <tar-dir> <out>" >&2
  exit 2
}

[[ $# -eq 2 ]] || usage
TAR_DIR="$1"
OUT="$2"
[[ -d "$TAR_DIR" ]] || { echo "✖ $TAR_DIR isn't a directory" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

: >"$OUT"
shopt -s nullglob
tar_files=("$TAR_DIR"/*.tar)
shopt -u nullglob
[[ ${#tar_files[@]} -gt 0 ]] || { echo "✖ $TAR_DIR has no *.tar files" >&2; exit 1; }

for tarfile in "${tar_files[@]}"; do
  date="$(basename "$tarfile" .tar)"
  tar -xf "$tarfile" -C "$WORK"
  manifest_path="$(find "$WORK/$date/release/oci/blobs/sha256" -type f ! -type l | head -n 1)"
  [[ -n "$manifest_path" ]] || { echo "✖ $date  no manifest blob in $tarfile" >&2; exit 1; }
  echo "$date sha256:$(basename "$manifest_path")" >>"$OUT"
done
