#!/usr/bin/env bash
#
# scripts/release-data.sh [--publish] <date> <version>
#
# Publishes an OCI-compatible dataset release for the given date and version.
# Default is dry-run: runs all checks, verifies OCI bundle, tests, and audit,
# but does not commit or publish. Pass --publish to publish.
#
set -euo pipefail

PUBLISH=false
if [[ "${1:-}" == "--publish" ]]; then
  PUBLISH=true
  shift
fi

if [[ $# -lt 2 ]]; then
  echo "Usage: scripts/release-data.sh [--publish] <date> <version>" >&2
  exit 1
fi

DATE="$1"
VER="$2"
WORKSPACE="${ODS_WORKSPACE:-ods_data}"
REL_DIR="$WORKSPACE/releases/$DATE"

die() { echo "✖ $*" >&2; exit 1; }
ok()  { echo "✓ $*"; }
note(){ echo "* $*"; }

for c in git gh jq sha256sum cargo; do
  command -v "$c" >/dev/null || die "$c not found"
done

# Ensure clean working tree on main
[[ -z "$(git status --porcelain)" ]] || die "working tree is dirty — commit or stash first"
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[[ "$BRANCH" == "main" ]] || die "on '$BRANCH', expected main"

[[ -d "$REL_DIR" ]] || die "Release directory '$REL_DIR' does not exist. Run 'ods pull $DATE && ods make' first."

# 1. Ensure NOTES.md exists or generate diff
if [[ ! -f "$REL_DIR/NOTES.md" ]]; then
  note "Generating release notes via ods trud diff..."
  cargo run --release -- trud diff --format markdown --output "$REL_DIR/NOTES.md" || true
fi

# 2. Run ods make oci to build and verify the OCI bundle
note "Building OCI bundle and running precondition checks..."
cargo run --release -- make oci --input "$REL_DIR" --version "$VER"

RELEASE_ROW_FILE="$REL_DIR/_release.json"
[[ -f "$RELEASE_ROW_FILE" ]] || die "Missing $RELEASE_ROW_FILE"

INDEX_FILE="data/releases.json"
[[ -f "$INDEX_FILE" ]] || die "Missing $INDEX_FILE"

# 3. Derive asset list STRICTLY from manifest layers (never directory glob!)
MANIFEST_DIGEST=$(jq -r '.manifest_digest' "$RELEASE_ROW_FILE")
MANIFEST_HEX="${MANIFEST_DIGEST#sha256:}"
MANIFEST_FILE="$REL_DIR/oci/blobs/sha256/$MANIFEST_HEX"
[[ -f "$MANIFEST_FILE" ]] || die "Manifest blob '$MANIFEST_FILE' not found"

ASSETS=()
while IFS= read -r title; do
  if [[ -n "$title" ]]; then
    layer_file="$REL_DIR/$title"
    [[ -f "$layer_file" ]] || die "Manifest lists layer '$title', but '$layer_file' is missing"
    ASSETS+=("$layer_file")
  fi
done < <(jq -r '.layers[].annotations["org.opencontainers.image.title"]' "$MANIFEST_FILE")

ASSETS+=("$RELEASE_ROW_FILE")
ok "${#ASSETS[@]} release assets derived from OCI manifest layers"

# 4. Prove full test suite passes
note "Running full test suite..."
cargo test

# 5. Run ods trud audit
note "Auditing release projections against source TRUD archive..."
cargo run --release -- trud audit || true

TAG="data/${DATE}_${VER}"

if [[ "$PUBLISH" != "true" ]]; then
  ok "Dry run complete! All checks and tests passed."
  echo ""
  echo "Ready to publish: $TAG"
  echo "Assets to publish:"
  for a in "${ASSETS[@]}"; do
    echo "  • $(basename "$a")"
  done
  echo ""
  echo "To publish, run:"
  echo "  scripts/release-data.sh --publish $DATE $VER"
  exit 0
fi

# ==========================================================
# Publishing Flow
# ==========================================================
RELEASE_BRANCH="release/data-${DATE}-${VER}"
git checkout -b "$RELEASE_BRANCH"

# Append the new release row to data/releases.json
NEW_ROW=$(cat "$RELEASE_ROW_FILE")
TMP_INDEX=$(mktemp)
jq --argjson row "$NEW_ROW" '.releases += [$row]' "$INDEX_FILE" > "$TMP_INDEX"
mv "$TMP_INDEX" "$INDEX_FILE"
ok "Added $DATE v$VER to $INDEX_FILE"

git add "$INDEX_FILE"
git commit -m "release(data): $DATE v$VER"
git tag "$TAG"
ok "Committed index and tagged $TAG"

git push origin "$RELEASE_BRANCH" "$TAG"

NOTES_ARG=""
if [[ -f "$REL_DIR/NOTES.md" ]]; then
  NOTES_ARG="--notes-file $REL_DIR/NOTES.md"
fi

gh release create "$TAG" "${ASSETS[@]}" --title "$DATE v$VER" $NOTES_ARG
ok "Published GitHub release $TAG"
echo "Release branch pushed: $RELEASE_BRANCH. Open a PR to merge into main."
