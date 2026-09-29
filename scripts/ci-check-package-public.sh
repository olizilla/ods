#!/usr/bin/env bash
#
# scripts/ci-check-package-public.sh <repository>
#
# Checks that ghcr.io/olizilla/<repository> can be read by a stranger: asks ghcr.io, with no
# credentials, for the package's tags, the same request an anonymous `ods pull` starts with.
# build-dataset.yml runs this first, so a run into an unreadable package fails in its first
# minute instead of after the ten-minute build. A maintainer can run it too, e.g. after making a
# package public, and wait until it passes before dispatching.
#
#   200                            ✓ public, exit 0
#   404 or NAME_UNKNOWN            ✖ the package doesn't exist, exit 1
#   401, 403 or DENIED             ✖ the package is private, exit 1
#   anything else                  ✖ names the status, exit 1
#
# One exception, observed on ghcr.io on 2026-09-29: ghcr.io refuses the pull *token* request
# itself for a package that isn't readable anonymously. It answers 403 DENIED for a name it
# doesn't know, and 401 UNAUTHORIZED for a private package. So at the token request, and only
# there, 403 DENIED means the package doesn't exist.
#
# A run never creates a dataset package (ghcr.io makes a new package private), so a package that
# doesn't exist is a failure here, not a first push.
#
# Needs curl and jq.
#
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: scripts/ci-check-package-public.sh <repository>" >&2
  exit 2
fi
REPOSITORY="$1"

# A ghcr.io package name: lowercase letters, digits, and . _ - only.
if [[ ! "$REPOSITORY" =~ ^[a-z0-9][a-z0-9._-]*$ ]]; then
  echo "✖ '$REPOSITORY' isn't a package name (lowercase letters, digits, '.', '_' and '-')" >&2
  exit 2
fi

for tool in curl jq; do
  command -v "$tool" >/dev/null 2>&1 || { echo "✖ $tool is needed and isn't installed" >&2; exit 2; }
done

IMAGE="ghcr.io/olizilla/$REPOSITORY"
body="$(mktemp)"
trap 'rm -f "$body"' EXIT

# The two ✖ blocks, so the token request and the tags request can both end in either.
not_found() {
  echo "✖ $IMAGE doesn't exist" >&2
  echo "  A run never creates a dataset package. Create it once, by hand:" >&2
  echo "    oras push $IMAGE:init README.md   (logged in with write:packages)" >&2
  echo "  then in its Package settings: Manage Actions access → olizilla/ods → Write, and make it Public." >&2
  exit 1
}
not_public() {
  echo "✖ $IMAGE isn't public: a stranger's \`ods pull\` couldn't read it" >&2
  echo "  Make it Public in its Package settings, then wait until this check passes before dispatching." >&2
  exit 1
}

# The registry's own error codes in the last response body, e.g. NAME_UNKNOWN, UNAUTHORIZED,
# DENIED. Empty when the body isn't a registry error.
error_codes() {
  jq -r '[.errors[]?.code] | join(" ")' "$body" 2>/dev/null || true
}

# Step 1: an anonymous pull token, the way any registry client starts.
token_url="https://ghcr.io/token?service=ghcr.io&scope=repository:olizilla/$REPOSITORY:pull"
status="$(curl -sS -o "$body" -w '%{http_code}' "$token_url")" || {
  echo "✖ couldn't reach ghcr.io to ask for a pull token" >&2
  exit 1
}
if [[ "$status" != "200" ]]; then
  codes="$(error_codes)"
  if [[ "$status" == "403" || "$status" == "404" || "$codes" == *DENIED* || "$codes" == *NAME_UNKNOWN* ]]; then
    not_found
  elif [[ "$status" == "401" || "$codes" == *UNAUTHORIZED* ]]; then
    not_public
  fi
  echo "✖ ghcr.io answered HTTP $status${codes:+ ($codes)} to the anonymous pull token request for $IMAGE" >&2
  exit 1
fi
token="$(jq -r '.token // empty' "$body")"
if [[ -z "$token" ]]; then
  echo "✖ ghcr.io's answer to the pull token request for $IMAGE had no token" >&2
  exit 1
fi

# Step 2: list the tags with that token.
status="$(curl -sS -o "$body" -w '%{http_code}' \
  -H "Authorization: Bearer $token" \
  "https://ghcr.io/v2/olizilla/$REPOSITORY/tags/list")" || {
  echo "✖ couldn't reach ghcr.io to list the tags of $IMAGE" >&2
  exit 1
}
codes="$(error_codes)"

if [[ "$status" == "200" ]]; then
  tag_count="$(jq -r '.tags | length' "$body")"
  echo "✓ $IMAGE is public ($tag_count tags)"
  exit 0
fi

if [[ "$status" == "404" || "$codes" == *NAME_UNKNOWN* ]]; then
  not_found
elif [[ "$status" == "401" || "$status" == "403" || "$codes" == *DENIED* || "$codes" == *UNAUTHORIZED* ]]; then
  not_public
fi

echo "✖ ghcr.io answered HTTP $status${codes:+ ($codes)} listing the tags of $IMAGE" >&2
exit 1
