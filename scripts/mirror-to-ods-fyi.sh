#!/usr/bin/env bash
#
# scripts/mirror-to-ods-fyi.sh <date> <version> [--repository ods-data] [--index <file>]
#                               [--from-oci-layout <dir>] [--publish]
#
# Copies one dataset from ghcr.io to ods.fyi's R2 bucket, digests intact:
#
#   1. `oras cp ghcr.io/olizilla/<repository>:<date>_<version> --to-oci-layout` into a scratch
#      directory, or use an already-pulled layout with --from-oci-layout <dir>;
#   2. check the manifest digest against --index <file>'s dataset row <date>_<version>, if given,
#      or just print it;
#   3. lay the layout out as bucket keys — the one place this repo lays out bucket keys:
#        v2/<repository>/blobs/sha256/<hex>
#        v2/<repository>/manifests/{<date>_<version>, <date>}
#      Never `latest`: that tag is what https://ods.fyi/orgs.parquet serves, and it moves with
#      releases.json, in scripts/bless-release.sh, so the two can't disagree.
#   4. with --publish, upload with `op run --env-file=.r2.env -- rclone copy`: blobs first, then
#      manifests, then re-verify every key from https://ods.fyi/v2/<repository>/…: each blob's
#      own SHA-256 against its name, and each manifest tag's bytes against the manifest digest. A
#      digest is only a promise until it's been fetched back.
#
# A dry run by default: it always prints the keys, in upload order, and only uploads (and
# re-verifies) with --publish. It never uploads releases.json or moves `latest` — blessing stays a
# deliberate, separate step, and this script ends by naming the script that does it.
#
set -euo pipefail

usage() {
  echo "usage: scripts/mirror-to-ods-fyi.sh <date> <version> [--repository ods-data] [--index <file>] [--from-oci-layout <dir>] [--publish]" >&2
  exit 2
}

[[ $# -ge 2 ]] || usage
DATE="$1"
VERSION="$2"
shift 2

REPOSITORY="ods-data"
INDEX=""
FROM_LAYOUT=""
PUBLISH=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repository) [[ $# -ge 2 ]] || usage; REPOSITORY="$2"; shift 2 ;;
    --index) [[ $# -ge 2 ]] || usage; INDEX="$2"; shift 2 ;;
    --from-oci-layout) [[ $# -ge 2 ]] || usage; FROM_LAYOUT="$2"; shift 2 ;;
    --publish) PUBLISH=true; shift ;;
    *) usage ;;
  esac
done

command -v jq >/dev/null 2>&1 || { echo "✖ jq is needed and isn't installed" >&2; exit 2; }

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

TAGGED="${DATE}_${VERSION}"

if [[ -n "$FROM_LAYOUT" ]]; then
  [[ -d "$FROM_LAYOUT" ]] || { echo "✖ $FROM_LAYOUT isn't a directory" >&2; exit 2; }
  LAYOUT="$FROM_LAYOUT"
else
  command -v oras >/dev/null 2>&1 || { echo "✖ oras is needed and isn't installed" >&2; exit 2; }
  LAYOUT="$WORK/layout"
  oras cp "ghcr.io/olizilla/$REPOSITORY:$TAGGED" --to-oci-layout "$LAYOUT:$TAGGED" >/dev/null
fi
[[ -f "$LAYOUT/index.json" ]] || { echo "✖ $LAYOUT isn't an OCI layout (no index.json)" >&2; exit 2; }

# 2. the manifest digest, from the layout's index.json entry tagged <date>_<version>
digest="$(jq -r --arg ref "$TAGGED" \
  '[.manifests[] | select(.annotations["org.opencontainers.image.ref.name"] == $ref)][0].digest // empty' \
  "$LAYOUT/index.json")"
[[ -n "$digest" ]] || { echo "✖ $LAYOUT/index.json has no manifest tagged $TAGGED" >&2; exit 1; }

if [[ -n "$INDEX" ]]; then
  [[ -f "$INDEX" ]] || { echo "✖ $INDEX isn't a file" >&2; exit 2; }
  row_digest="$(jq -r --arg d "$DATE" --arg v "$TAGGED" \
    '[.releases[] | select(.source.version == $d) | .datasets[] | select(.version == $v)][0].manifest_digest // empty' \
    "$INDEX")"
  if [[ -z "$row_digest" ]]; then
    echo "✖ $INDEX has no row for $TAGGED" >&2
    exit 1
  fi
  if [[ "$row_digest" != "$digest" ]]; then
    echo "✖ the manifest digest doesn't match $INDEX's row for $TAGGED" >&2
    echo "  the layout  $digest" >&2
    echo "  the index   $row_digest" >&2
    exit 1
  fi
  echo "✓ manifest $digest matches $INDEX's row for $TAGGED"
else
  echo "* manifest $digest"
fi

manifest_hex="${digest#sha256:}"
manifest_path="$LAYOUT/blobs/sha256/$manifest_hex"
[[ -f "$manifest_path" ]] || { echo "✖ $LAYOUT has no blob for $digest" >&2; exit 1; }

# 3. lay the layout out as bucket keys
BUCKET="$WORK/bucket"
mkdir -p "$BUCKET/v2/$REPOSITORY/blobs/sha256" "$BUCKET/v2/$REPOSITORY/manifests"

: >"$WORK/blob-keys.txt"
echo "$manifest_hex" >>"$WORK/blob-keys.txt"
jq -r '.layers[].digest' "$manifest_path" | sed 's/^sha256://' >>"$WORK/blob-keys.txt"

while read -r hex; do
  [[ -f "$LAYOUT/blobs/sha256/$hex" ]] || { echo "✖ $LAYOUT is missing blob $hex" >&2; exit 1; }
  cp "$LAYOUT/blobs/sha256/$hex" "$BUCKET/v2/$REPOSITORY/blobs/sha256/$hex"
  echo "v2/$REPOSITORY/blobs/sha256/$hex"
done <"$WORK/blob-keys.txt" >"$WORK/upload-order.txt"

for ref in "$TAGGED" "$DATE"; do
  cp "$manifest_path" "$BUCKET/v2/$REPOSITORY/manifests/$ref"
  echo "v2/$REPOSITORY/manifests/$ref" >>"$WORK/upload-order.txt"
done

echo "* upload order:"
sed 's/^/  /' "$WORK/upload-order.txt"

if [[ "$PUBLISH" == true ]]; then
  echo "* uploading blobs…"
  op run --env-file=.r2.env -- rclone copy "$BUCKET/v2/$REPOSITORY/blobs" "r2:ods-fyi/v2/$REPOSITORY/blobs"
  echo "* uploading manifests…"
  op run --env-file=.r2.env -- rclone copy "$BUCKET/v2/$REPOSITORY/manifests" "r2:ods-fyi/v2/$REPOSITORY/manifests"
  echo "✓ published $TAGGED to ods.fyi"

  command -v curl >/dev/null 2>&1 || { echo "✖ curl is needed and isn't installed" >&2; exit 2; }
  echo "* re-verifying every key from https://ods.fyi/…"
  reverify_ok=true
  while IFS= read -r key; do
    url="https://ods.fyi/$key"
    actual="$(curl -fsSL "$url" | shasum -a 256 | cut -d' ' -f1)"
    if [[ "$key" == */manifests/* ]]; then
      expected="$manifest_hex"
    else
      expected="${key##*/}"
    fi
    if [[ "$actual" == "$expected" ]]; then
      echo "✓ $key"
    else
      echo "✖ $key" >&2
      echo "  expected  $expected" >&2
      echo "  got       $actual" >&2
      reverify_ok=false
    fi
  done <"$WORK/upload-order.txt"
  [[ "$reverify_ok" == true ]] || exit 1
else
  echo "* a dry run: pass --publish to upload"
  echo "* --publish would then re-verify every key above from https://ods.fyi/…: each blob's own"
  echo "  SHA-256 against its name, and each manifest tag's bytes against $digest"
fi

echo "* releases.json and the latest tag are never touched here. To bless this release, commit"
echo "  the index as data/releases.json, then run: scripts/bless-release.sh --publish"
