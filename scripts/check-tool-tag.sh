#!/usr/bin/env bash
#
# scripts/check-tool-tag.sh
#
# Fails unless run from a checkout of the tag v<Cargo.toml version>, run from the repo root.
# Every job of build-dataset.yml, monthly-dataset.yml and rebuild-datasets.yml runs this first,
# right after checkout: the tag gives the clean build at a known commit that the index row
# (tool_git_sha) and the attestation record, so nothing here is built, pushed or attested from
# a branch. Reads GITHUB_REF_TYPE and GITHUB_REF_NAME, which Actions sets for every job; a local
# run can set them the same way to check the logic.
#
set -euo pipefail

VERSION="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' Cargo.toml | head -1)"
[[ -n "$VERSION" ]] || { echo "✖ can't read the tool version from Cargo.toml" >&2; exit 1; }
TAG="v$VERSION"

if [[ "${GITHUB_REF_TYPE:-}" != "tag" || "${GITHUB_REF_NAME:-}" != "$TAG" ]]; then
  echo "✖ this workflow only runs from the tag $TAG (Cargo.toml's version), workflow_dispatch only" >&2
  echo "  it ran from ${GITHUB_REF_TYPE:-<unset>} '${GITHUB_REF_NAME:-<unset>}'" >&2
  exit 1
fi

echo "✓ running from tag $TAG"
