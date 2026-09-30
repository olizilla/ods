#!/usr/bin/env bash
#
# scripts/bless-release.sh [--publish]
#
# Makes data/releases.json the index ods.fyi serves, and points ods.fyi's `latest` tag at the
# newest release in it. The two change together, here and nowhere else, so
# https://ods.fyi/orgs.parquet (which serves `latest`) is never ahead of the index:
#
#   1. data/releases.json must be committed: git holds the canonical copy, so it goes first.
#   2. The newest release is the greatest source version with a dataset that isn't withdrawn,
#      and its dataset is the last one that isn't withdrawn (they're ordered oldest first).
#   3. That dataset must already be on ods.fyi (scripts/mirror-to-ods-fyi.sh): its manifest is
#      fetched back by digest and checked, and each of its layers must answer.
#   4. With --publish: upload releases.json, then copy the manifest to
#      v2/<name>/manifests/latest, in that order, so a failure in between leaves `latest` behind
#      the index, never ahead of it. Then fetch both back and check them.
#
# A dry run by default: it does steps 1 to 3 and says what --publish would do.
#
set -euo pipefail

usage() {
  echo "usage: scripts/bless-release.sh [--publish]" >&2
  exit 2
}

PUBLISH=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --publish) PUBLISH=true; shift ;;
    *) usage ;;
  esac
done

for tool in jq curl git shasum; do
  command -v "$tool" >/dev/null 2>&1 || { echo "✖ $tool is needed and isn't installed" >&2; exit 2; }
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"
INDEX="data/releases.json"

# 1. the index is the committed one
if ! git diff --quiet HEAD -- "$INDEX"; then
  echo "✖ $INDEX has changes that aren't committed" >&2
  echo "  Git holds the canonical copy. Commit it, then run this again." >&2
  exit 1
fi

# 2. the newest release, and its dataset
name="$(jq -r '.name' "$INDEX")"
newest="$(jq -c '
  [.releases[] | select(any(.datasets[]; .withdrawn == null))]
  | max_by(.source.version)
  | {date: .source.version, dataset: ([.datasets[] | select(.withdrawn == null)] | last)}
' "$INDEX")"
date="$(jq -r '.date // empty' <<<"$newest")"
if [[ -z "$date" ]]; then
  echo "✖ $INDEX lists no dataset that isn't withdrawn, so there is nothing for latest to name" >&2
  exit 1
fi
tagged="$(jq -r '.dataset.version' <<<"$newest")"
digest="$(jq -r '.dataset.manifest_digest' <<<"$newest")"
hex="${digest#sha256:}"
echo "* newest release in $INDEX: $tagged  $digest"

# 3. it's on ods.fyi already
base="https://ods.fyi/v2/$name"
manifest="$(mktemp)"
trap 'rm -f "$manifest"' EXIT
not_mirrored() {
  echo "✖ $tagged isn't all on ods.fyi: $1" >&2
  echo "  Mirror it first: scripts/mirror-to-ods-fyi.sh $date ${tagged#"${date}"_} --index $INDEX --publish" >&2
  exit 1
}
curl -fsSL -o "$manifest" "$base/blobs/$digest" || not_mirrored "its manifest, $base/blobs/$digest, didn't download"
actual="$(shasum -a 256 "$manifest" | cut -d' ' -f1)"
[[ "$actual" == "$hex" ]] || not_mirrored "its manifest hashes to sha256:$actual, not $digest"
while IFS= read -r layer; do
  curl -fsSI -o /dev/null "$base/blobs/$layer" || not_mirrored "the layer $layer doesn't answer"
done < <(jq -r '.layers[].digest' "$manifest")
echo "✓ $tagged is on ods.fyi: its manifest matches and every layer answers"

if [[ "$PUBLISH" != true ]]; then
  echo "* a dry run: pass --publish to upload $INDEX as releases.json,"
  echo "  then point v2/$name/manifests/latest at $tagged"
  exit 0
fi

# 4. the index, then latest
command -v op >/dev/null 2>&1 || { echo "✖ op is needed and isn't installed" >&2; exit 2; }
command -v rclone >/dev/null 2>&1 || { echo "✖ rclone is needed and isn't installed" >&2; exit 2; }

echo "* uploading releases.json…"
op run --env-file=.r2.env -- rclone copyto "$INDEX" r2:ods-fyi/releases.json
echo "* pointing latest at $tagged…"
op run --env-file=.r2.env -- rclone copyto "$manifest" "r2:ods-fyi/v2/$name/manifests/latest"

echo "* checking both from https://ods.fyi/…"
ok=true
if curl -fsSL https://ods.fyi/releases.json | cmp -s - "$INDEX"; then
  echo "✓ releases.json is $INDEX"
else
  echo "✖ https://ods.fyi/releases.json isn't $INDEX" >&2
  ok=false
fi
served="$(curl -fsSL "$base/manifests/latest" | shasum -a 256 | cut -d' ' -f1)"
if [[ "$served" == "$hex" ]]; then
  echo "✓ latest is $tagged"
else
  echo "✖ https://ods.fyi/v2/$name/manifests/latest hashes to sha256:$served, not $digest" >&2
  ok=false
fi
[[ "$ok" == true ]] || exit 1
echo "✓ blessed: ods.fyi lists and serves $tagged as its newest release"
