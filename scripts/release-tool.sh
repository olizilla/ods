#!/usr/bin/env bash
#
# scripts/release-tool.sh [--publish]
#
# Tags and releases a new version of the ods CLI tool.
# Default is dry-run: checks working tree, branch, tests, and CHANGELOG.md.
# Pass --publish to tag and create the GitHub release.
#
set -euo pipefail

PUBLISH=false
if [[ "${1:-}" == "--publish" ]]; then
  PUBLISH=true
fi

die() { echo "✖ $*" >&2; exit 1; }
ok()  { echo "✓ $*"; }
note(){ echo "* $*"; }

for c in git cargo grep gh sed; do
  command -v "$c" >/dev/null || die "$c not found"
done

# 1. Refuse on dirty tree or a branch other than main
[[ -z "$(git status --porcelain)" ]] || die "working tree is dirty — commit or stash first"
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
[[ "$BRANCH" == "main" ]] || die "on '$BRANCH', expected main branch"

# 2. Read version from Cargo.toml
VERSION=$(grep -m1 '^version = ' Cargo.toml | cut -d '"' -f 2)
TAG="v$VERSION"

# 3. Refuse if tag already exists locally or on origin
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && die "Tag $TAG already exists locally"
if git remote get-url origin >/dev/null 2>&1; then
  if git ls-remote --exit-code --tags origin "$TAG" >/dev/null 2>&1; then
    die "Tag $TAG already exists on origin"
  fi
fi

# 4. Refuse if CHANGELOG.md has no entry for this version
[[ -f "CHANGELOG.md" ]] || die "Missing CHANGELOG.md"
if ! grep -q -E "^## \[(v)?${VERSION}\]" CHANGELOG.md; then
  die "CHANGELOG.md has no entry for version $VERSION (expected '## [$VERSION]' or '## [v$VERSION]')"
fi

# Extract changelog section for notes
CHANGELOG_NOTES=$(awk -v ver="$VERSION" '
  $0 ~ "^## \\[(v)?" ver "\\]" { found=1; next }
  found && /^## / { found=0 }
  found { print }
' CHANGELOG.md | sed -e :a -e '/^\n*$/{$d;N;};/\n$/ba')

[[ -n "$CHANGELOG_NOTES" ]] || die "Changelog section for $VERSION is empty"
ok "Found release notes in CHANGELOG.md for $VERSION"

# 5. Refuse if cargo test fails
note "Running cargo test..."
cargo test

if [[ "$PUBLISH" != "true" ]]; then
  ok "Dry run complete! Ready to release $TAG"
  echo ""
  echo "Release notes:"
  echo "$CHANGELOG_NOTES"
  echo ""
  echo "To release tool, run:"
  echo "  scripts/release-tool.sh --publish"
  exit 0
fi

# Tag, push and release
note "Tagging $TAG..."
git tag "$TAG"
ok "Tagged $TAG"

git push origin "$TAG"
ok "Pushed $TAG to origin"

gh release create "$TAG" --title "ods $TAG" --notes "$CHANGELOG_NOTES"
ok "Published GitHub release $TAG"
