#!/usr/bin/env bash
#
# scripts/ci-push-dataset.sh <repository> <version> <date> <gated-digest> <oci-layout-dir> <digest-out>
#
# build-dataset.yml's primary job, one date at a time, right after primary has checked its own
# digest equals the witness's. Pushes <oci-layout-dir>'s manifest — the primary's own build,
# already gated equal to the witness's — to ghcr.io/olizilla/<repository>:<date>_<version> and
# :<date>.
#
# Push only, under the tag rule — a pushed versioned tag is never moved. `oras repo tags` lists
# what's already there; membership in that list is the only thing that means "exists" — a failing
# call is a failure, never read as "absent":
#   - the versioned tag is listed, and resolves to the gated digest   → already pushed, skip
#   - the versioned tag is listed, and resolves to something else    → refused, a ✖ block, exit 1
#   - the versioned tag isn't listed                                 → pushed
# The bare <date> tag is a convenience pointer, not a release object, so it always tracks the
# gated digest, unconditionally re-pushed.
#
# Re-verifying what was pushed is the caller's job now (`ods pull --index`, against the candidate
# row this push produced) — not this script's. Writes the gated digest to <digest-out> once the
# push has landed (for the caller to put in $GITHUB_OUTPUT). Needs oras and a working ghcr.io
# login already done by the caller (docker/login-action; oras reads the same credential store).
#
set -euo pipefail

usage() {
  echo "usage: scripts/ci-push-dataset.sh <repository> <version> <date> <gated-digest> <oci-layout-dir> <digest-out>" >&2
  exit 2
}

[[ $# -eq 6 ]] || usage
REPOSITORY="$1"
VERSION="$2"
DATE="$3"
GATED_DIGEST="$4"
LAYOUT="$5"
DIGEST_OUT="$6"

command -v oras >/dev/null 2>&1 || { echo "✖ oras is needed and isn't installed" >&2; exit 2; }
[[ -d "$LAYOUT" ]] || { echo "✖ $LAYOUT isn't a directory" >&2; exit 2; }

VERSIONED_TAG="${DATE}_${VERSION}"
REF="ghcr.io/olizilla/$REPOSITORY"

existing_tags="$(oras repo tags "$REF")" || {
  echo "✖ $DATE  listing tags for $REF failed, so whether $VERSIONED_TAG exists is unknown" >&2
  exit 1
}

if grep -qx "$VERSIONED_TAG" <<<"$existing_tags"; then
  remote_digest="$(oras resolve "$REF:$VERSIONED_TAG")"
  if [[ "$remote_digest" == "$GATED_DIGEST" ]]; then
    echo "✓ $DATE  $REF:$VERSIONED_TAG already names $GATED_DIGEST — skipping the copy"
  else
    echo "✖ $DATE  $REF:$VERSIONED_TAG already names a different digest" >&2
    echo "  the registry  $remote_digest" >&2
    echo "  this build    $GATED_DIGEST" >&2
    echo "  A pushed tag is never moved. If this date is being rebuilt with different bytes, that is news to report." >&2
    exit 1
  fi
else
  oras cp --from-oci-layout "$LAYOUT:$VERSIONED_TAG" "$REF:$VERSIONED_TAG" >/dev/null
  echo "✓ $DATE  pushed $REF:$VERSIONED_TAG  $GATED_DIGEST"
fi

# The bare <date> tag always tracks the gated digest — a convenience pointer, not the release
# object, so unlike the versioned tag it's never checked first, only ever set.
oras cp --from-oci-layout "$LAYOUT:$DATE" "$REF:$DATE" >/dev/null

pushed_digest="$(oras resolve "$REF:$DATE")"
if [[ "$pushed_digest" != "$GATED_DIGEST" ]]; then
  echo "✖ $DATE  $REF:$DATE resolves to $pushed_digest, not the gated digest $GATED_DIGEST" >&2
  exit 1
fi
echo "✓ $DATE  $REF:$DATE names $pushed_digest"

echo "$GATED_DIGEST" >"$DIGEST_OUT"
