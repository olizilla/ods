#!/usr/bin/env bash
#
# scripts/push-source-bundles.sh [-w <workspace>] [--push] [--repository nhs-ods-xml]
#
# Pushes each release's trud/oci/ (packed by `ods make oci --source`) to
# ghcr.io/olizilla/<repository>:<date>, once. For each release, in order:
#
#   - scripts/verify-trud-bundle.sh checks NHS's signature against the key pinned in
#     data/releases.json, and that the signed checksum is the zip's;
#   - a tag that already names this digest is skipped;
#   - a tag that names a different digest is refused, and nothing is pushed for that date: if NHS
#     ever republishes a date, that is news to report, not a tag to overwrite.
#
# A dry run by default; --push pushes. Logs in with `gh auth token`, which needs the
# write:packages scope (`gh auth refresh -s write:packages`).
#
# Each bundle's manifest carries the annotation fyi.ods.trud-release-sha256: a convenience copy of
# the zip layer's digest, in upper case, in the same form as the dataset manifests' annotation, so
# a dataset links to its source bundle by one value.
#
# The package is meant to stay private and is published and forgotten: a bundle holds exactly
# NHS's bytes and terms, its digest is a fixed function of them, and a tag is never moved.
# Visibility is set in the package's settings, not here, and this script doesn't police it.
#
# One-time setup, in the package's settings on GitHub: under "Manage Actions access", give
# olizilla/ods read access, so CI can pull the bundles.
#
# Restore a release from the registry, with NHS's four files under their own names in <dir>/trud:
#   oras cp ghcr.io/olizilla/nhs-ods-xml:<date> --to-oci-layout <dir>:<date>
#   then read the layers out of <dir>/blobs/sha256/, or `oras pull ghcr.io/olizilla/nhs-ods-xml:<date> -o <dir>`
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE="ods_data"
REPOSITORY="nhs-ods-xml"
OWNER="olizilla"
PUSH=false

usage() {
  echo "usage: scripts/push-source-bundles.sh [-w <workspace>] [--push] [--repository nhs-ods-xml]" >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    -w|--workspace) [[ $# -ge 2 ]] || usage; WORKSPACE="$2"; shift 2 ;;
    --repository) [[ $# -ge 2 ]] || usage; REPOSITORY="$2"; shift 2 ;;
    --push) PUSH=true; shift ;;
    *) usage ;;
  esac
done

for tool in oras gh jq; do
  command -v "$tool" >/dev/null 2>&1 || { echo "✖ $tool is needed and isn't installed" >&2; exit 2; }
done
[[ -d "$WORKSPACE/releases" ]] || { echo "✖ $WORKSPACE has no releases/ directory" >&2; exit 2; }

# Log in with the token gh holds, and say what to run when it lacks the scope
if ! gh auth status 2>&1 | grep -q 'write:packages'; then
  echo "✖ the gh token can't push packages" >&2
  echo "  Run: gh auth refresh -s write:packages" >&2
  exit 2
fi
gh auth token | oras login ghcr.io -u "$(gh api user -q .login)" --password-stdin >/dev/null

pushed=0
skipped=0
refused=0
failed=0

# Newest first, one release at a time
for release_dir in $(find "$WORKSPACE/releases" -mindepth 1 -maxdepth 1 -type d | sort -r); do
  date="$(basename "$release_dir")"
  trud_dir="$release_dir/trud"
  bundle="$trud_dir/oci"
  ref="ghcr.io/$OWNER/$REPOSITORY:$date"

  if [[ ! -f "$bundle/index.json" ]]; then
    echo "✖ $date  nothing packed at $bundle" >&2
    echo "  Pack it with: ods make oci --source -i $release_dir" >&2
    failed=$((failed + 1))
    continue
  fi

  if ! verified="$("$REPO_ROOT/scripts/verify-trud-bundle.sh" "$trud_dir" "$REPO_ROOT/data/releases.json" 2>&1)"; then
    echo "$verified" >&2
    echo "  Nothing is pushed for $date." >&2
    failed=$((failed + 1))
    continue
  fi

  local_digest="$(jq -r '.manifests[0].digest' "$bundle/index.json")"

  # Only oras's not-found error means the tag is absent. Any other failure (auth, network, a rate
  # limit) fails this date, and nothing is pushed for it.
  lookup_error="$(mktemp)"
  if remote_digest="$(oras resolve "$ref" 2>"$lookup_error")"; then
    :
  elif grep -Eq ': not found[[:space:]]*$' "$lookup_error"; then
    remote_digest=""
  else
    echo "✖ $date  looking up $ref failed, so whether the tag exists is unknown" >&2
    sed 's/^/  /' "$lookup_error" >&2
    echo "  Nothing is pushed for $date." >&2
    rm -f "$lookup_error"
    failed=$((failed + 1))
    continue
  fi
  rm -f "$lookup_error"

  if [[ "$remote_digest" == "$local_digest" ]]; then
    skipped=$((skipped + 1))
    continue
  fi
  if [[ -n "$remote_digest" ]]; then
    echo "✖ $date  $ref already names a different digest" >&2
    echo "  the registry  $remote_digest" >&2
    echo "  this bundle   $local_digest" >&2
    echo "  A pushed tag is never moved. If NHS republished $date, that is news to report." >&2
    refused=$((refused + 1))
    continue
  fi

  if [[ "$PUSH" == true ]]; then
    oras cp --from-oci-layout "$bundle:$date" "$ref" >/dev/null
    echo "✓ $date  pushed $ref  $local_digest"
  else
    echo "* $date  would push $ref  $local_digest"
  fi
  pushed=$((pushed + 1))
done

verb="would push"
[[ "$PUSH" == true ]] && verb="pushed"
sigil="✓"
[[ $((refused + failed)) -eq 0 ]] || sigil="✖"
echo "$sigil $pushed $verb · $skipped skipped · $refused refused · $failed failed"
[[ $((refused + failed)) -eq 0 ]] || exit 1
[[ "$PUSH" == true ]] || echo "* a dry run: pass --push to push"
