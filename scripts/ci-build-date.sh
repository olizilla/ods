#!/usr/bin/env bash
#
# scripts/ci-build-date.sh <date> <bundle-digest> <release-dir>
#
# The one per-date procedure build-dataset.yml's witness and primary both run, so a reader can see
# they're identical: pulls ghcr.io/olizilla/nhs-ods-xml@<bundle-digest> into a fresh trud/<date>,
# checks NHS's signature again (scripts/verify-trud-bundle.sh), then builds it exactly as a user's
# own `ods make <zip>` would: `ods make -i <zip> -o <release-dir>`, with no `--index` and no
# `--force` — `ods make` matches the zip's SHA-256 against the index built into this build (the
# tag's data/releases.json) first, then TRUD's API if TRUD_API_KEY is set, and writes
# `_provenance.json` from whichever matched. If neither knows this archive, `ods make` refuses on
# its own `✖`, and `set -e` stops this script there: a dataset built without provenance can't be
# pushed, attested or recorded. Then `ods make oci --format json` on the result, for its manifest
# digest.
#
# Prints the manifest digest as the last line of stdout; everything before it is `ods`'s and
# `oras`'s own progress, kept visible rather than suppressed (`ods make oci`'s own report moves to
# stderr under `--format json`, for the same reason). Wraps each phase in ::group:: lines when
# GITHUB_ACTIONS=true, so verification stays visible in the log. Needs `ods` on PATH (or
# ODS_BIN naming it), oras, jq, and TRUD_API_KEY in the environment to get
# provenance for a date that isn't in this build's own index yet.
#
set -euo pipefail

usage() {
  echo "usage: scripts/ci-build-date.sh <date> <bundle-digest> <release-dir>" >&2
  exit 2
}

[[ $# -eq 3 ]] || usage
DATE="$1"
BUNDLE_DIGEST="$2"
RELEASE_DIR="$3"

ODS="${ODS_BIN:-ods}"
command -v "$ODS" >/dev/null 2>&1 || { echo "✖ $ODS isn't installed. Run: cargo install --locked --path ." >&2; exit 2; }
command -v oras >/dev/null 2>&1 || { echo "✖ oras is needed and isn't installed" >&2; exit 2; }
command -v jq >/dev/null 2>&1 || { echo "✖ jq is needed and isn't installed" >&2; exit 2; }

group() {
  if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then echo "::group::$1"; else echo "* $1"; fi
}
endgroup() { [[ "${GITHUB_ACTIONS:-}" != "true" ]] || echo "::endgroup::"; }

TRUD_DIR="trud/$DATE"

group "Pull nhs-ods-xml $DATE"
rm -rf "$TRUD_DIR"
oras pull "ghcr.io/olizilla/nhs-ods-xml@$BUNDLE_DIGEST" -o "$TRUD_DIR" >/dev/null
endgroup

group "Verify source: nhs-ods-xml $DATE"
scripts/verify-trud-bundle.sh "$TRUD_DIR"
endgroup

zip_path="$(find "$TRUD_DIR" -maxdepth 1 -name '*.zip' | head -n 1)"
[[ -n "$zip_path" ]] || { echo "✖ no zip in $TRUD_DIR" >&2; exit 2; }

group "Build $DATE"
"$ODS" make -i "$zip_path" -o "$RELEASE_DIR"
endgroup

group "Pack $DATE"
digest="$("$ODS" make oci -i "$RELEASE_DIR" --format json | jq -r .manifest_digest)"
endgroup

echo "$digest"
