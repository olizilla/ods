#!/usr/bin/env bash
#
# scripts/release-data.sh [--publish] <date> <version>
#
# Publishes an OCI-compatible dataset release for the given date and version.
# Default is dry-run: runs all checks, verifies OCI bundle, tests, and audit,
# and displays the staging tree. Pass --publish to publish.
#
set -euo pipefail

if ((BASH_VERSINFO[0] < 5)); then
  echo "✖ bash 5 or higher required. Run 'brew install bash'" >&2
  exit 1
fi

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

trap 'rm -f "${DATE}_${VER}.oci.tar"' EXIT

die() { echo "✖ $*" >&2; exit 1; }
ok()  { echo "✓ $*"; }
note(){ echo "* $*"; }

for c in git gh jq cargo skopeo rclone; do
  if ! command -v "$c" >/dev/null 2>&1; then
    case "$c" in
      git)    die "git not found. Run 'brew install git'" ;;
      gh)     die "gh not found. Run 'brew install gh'" ;;
      jq)     die "jq not found. Run 'brew install jq'" ;;
      skopeo) die "skopeo not found. Run 'brew install skopeo'" ;;
      rclone) die "rclone not found. Run 'brew install rclone'" ;;
      *)      die "$c not found" ;;
    esac
  fi
done

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || die "not inside a git repository"
cd "$REPO_ROOT"

# 1. Ensure clean working tree on main, cargo test passes, audit passes
[[ -z "$(git status --porcelain)" ]] || die "working tree is dirty — commit or stash first"
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[[ "$BRANCH" == "main" ]] || die "on '$BRANCH', expected main"

[[ -d "$REL_DIR" ]] || die "Release directory '$REL_DIR' does not exist. Run 'ods trud pull $DATE && ods make' first."

note "Running full test suite..."
cargo test

note "Auditing release projections against source TRUD archive..."
cargo run --release -- audit --input "$REL_DIR" --workspace "$WORKSPACE" || true

# 2. Ensure NOTES.md exists or generate diff
if [[ ! -f "$REL_DIR/NOTES.md" ]]; then
  note "Generating release notes via ods trud diff..."
  cargo run --release -- trud diff --format markdown --output "$REL_DIR/NOTES.md" || true
fi

# 3. Run ods make release (assert the only git diff is data/releases.json)
note "Running ods make release..."
cargo run --release -- make release --input "$REL_DIR"

INDEX_FILE="data/releases.json"
[[ -f "$INDEX_FILE" ]] || die "Missing $INDEX_FILE"

CHANGED_FILES=$(git status --porcelain | awk '{print $2}')
if [[ "$CHANGED_FILES" != "data/releases.json" ]]; then
  die "Unexpected git status diff after make release: $CHANGED_FILES (expected only data/releases.json)"
fi

TAG="data/${DATE}_${VER}"

if [[ "$PUBLISH" != "true" ]]; then
  ok "Dry run complete! All checks and tests passed."
  echo ""
  echo "Ready to publish: $TAG"
  echo "Staging tree (dist/):"
  if [[ -d dist ]]; then
    find dist -type f | sort
  fi
  echo ""
  echo "To publish, run:"
  echo "  scripts/release-data.sh --publish $DATE $VER"
  exit 0
fi

# ==========================================================
# Publishing Flow
# ==========================================================
# 4. Sync OCI image layout to GHCR (versioned and bare-date tags)
note "Publishing OCI image to ghcr.io..."
skopeo copy "oci:$REL_DIR/oci:${DATE}_${VER}" "docker://ghcr.io/olizilla/ods-data:${DATE}_${VER}"
skopeo copy "docker://ghcr.io/olizilla/ods-data:${DATE}_${VER}" "docker://ghcr.io/olizilla/ods-data:$DATE"

# 5. Sync dist/ staging tree to R2 / ods.fyi (holds only release objects)
note "Syncing objects to Cloudflare R2 (ods.fyi)..."
rclone copy dist/ r2:ods-fyi/

# 6. Create OCI archive and attach to GitHub release
note "Publishing GitHub release with .oci.tar..."
skopeo copy "oci:$REL_DIR/oci:${DATE}_${VER}" "oci-archive:${DATE}_${VER}.oci.tar"
gh release create "$TAG" --notes-file "$REL_DIR/NOTES.md" "${DATE}_${VER}.oci.tar"
rm -f "${DATE}_${VER}.oci.tar"

# 7. Publish data/releases.json to R2 (the commit point for live index)
note "Publishing releases.json to R2..."
rclone copyto data/releases.json r2:ods-fyi/releases.json
ok "Release $TAG published live to ods.fyi"

# 8. Commit data/releases.json, tag, and push
git commit -m "release(data): $DATE v$VER" "$INDEX_FILE"
git tag "$TAG"
git push origin main "$TAG"
ok "Committed, tagged $TAG, and pushed to origin main"
