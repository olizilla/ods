#!/usr/bin/env bash
#
# scripts/ci-push-dataset.sh <repository> <version> <date> <gated-digest> <oci-layout-dir> <digest-out>
#
# build-dataset.yml's `push` job, one date at a time (its matrix runs this once per date, after
# `gate`). Pushes <oci-layout-dir>'s manifest — already gated equal on both runners — to
# ghcr.io/olizilla/<repository>:<date>_<version> and :<date>.
#
# A pushed versioned tag is never moved: `oras resolve` checks what's there first.
#   - the same digest as gated           → already pushed; the copy is skipped
#   - a different digest                 → refused, a ✖ block naming both, exit 1
#   - not found (oras's own not-found)   → pushed
# Any other lookup failure (auth, network, a rate limit) also refuses, since whether the tag
# exists is unknown. The bare <date> tag is a convenience pointer, not a release object, so it
# always tracks the gated digest, unconditionally re-pushed.
#
# Writes the pushed digest to <digest-out> (for the caller to put in $GITHUB_OUTPUT); status
# lines go to stdout/stderr as usual. Needs oras and a working `oras login` already done by the
# caller.
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

lookup_error="$(mktemp)"
trap 'rm -f "$lookup_error"' EXIT

remote_digest=""
if resolved="$(oras resolve "$REF:$VERSIONED_TAG" 2>"$lookup_error")"; then
  remote_digest="$resolved"
elif grep -Eq ': not found[[:space:]]*$' "$lookup_error"; then
  remote_digest=""
else
  echo "✖ $DATE  looking up $REF:$VERSIONED_TAG failed, so whether the tag exists is unknown" >&2
  sed 's/^/  /' "$lookup_error" >&2
  exit 1
fi

if [[ -n "$remote_digest" && "$remote_digest" == "$GATED_DIGEST" ]]; then
  echo "✓ $DATE  $REF:$VERSIONED_TAG already names $GATED_DIGEST — skipping the copy"
elif [[ -n "$remote_digest" ]]; then
  echo "✖ $DATE  $REF:$VERSIONED_TAG already names a different digest" >&2
  echo "  the registry  $remote_digest" >&2
  echo "  this build    $GATED_DIGEST" >&2
  echo "  A pushed tag is never moved. If this date is being rebuilt with different bytes, that is news to report." >&2
  exit 1
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
echo "$pushed_digest" >"$DIGEST_OUT"
