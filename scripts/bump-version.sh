#!/usr/bin/env bash
#
# scripts/bump-version.sh
# scripts/bump-version.sh tool <x.y.z>
# scripts/bump-version.sh dataset <x.y.z>
#
# With no arguments, prints the current tool and dataset versions.
#
# Each version is set in one place:
#   tool     Cargo.toml's [package] version (Cargo.lock follows)
#   dataset  src/datapackage.rs's DATASET_VERSION (data/datapackage.json follows)
#
# After a bump it lists lines outside tests/ and worker/test/ that still name the old version.
# Edit CHANGELOG.md yourself; scripts/release-tool.sh checks it has an entry.
#
# The two versions are linked (docs/tests.md D5): a dataset bump needs a tool bump at least as
# large, and each CHANGELOG entry names the dataset version its tool builds. Each bump prints
# the half of that rule it is about.
#
set -euo pipefail

die() { echo "✖ $*" >&2; exit 1; }
ok()  { echo "✓ $*"; }
note(){ echo "* $*"; }

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || die "not inside a git repository"
cd "$REPO_ROOT"

CARGO_TOML="Cargo.toml"
DATASET_RS="src/datapackage.rs"
DATAPACKAGE_JSON="data/datapackage.json"
EXPECTED_KEYS="worker/test/fixtures/expected-keys.json"

tool_version() {
  # The first `version = "…"` in Cargo.toml is [package]'s.
  sed -n 's/^version = "\([^"]*\)".*/\1/p' "$CARGO_TOML" | head -1
}

dataset_version() {
  sed -n 's/^pub const DATASET_VERSION: &str = "\([^"]*\)";/\1/p' "$DATASET_RS" | head -1
}

is_semver() { [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; }

# major, minor or patch: the largest part that differs between two versions
bump_size() {
  local from_major from_minor to_major to_minor _
  IFS=. read -r from_major from_minor _ <<<"$1"
  IFS=. read -r to_major to_minor _ <<<"$2"
  if [[ "$from_major" != "$to_major" ]]; then echo major
  elif [[ "$from_minor" != "$to_minor" ]]; then echo minor
  else echo patch
  fi
}

report_leftovers() {
  local old="$1"
  local hits tests
  hits=$(git grep -n -F "$old" -- . \
    ':!*.lock' ':!*package-lock.json' ':!CHANGELOG.md' ':!*/package.json' \
    ':!tests/' ':!worker/test/' || true)
  tests=$( (git grep -n -F "$old" -- tests/ worker/test/ 2>/dev/null || true) | wc -l | tr -d ' ' )
  if [[ -z "$hits" ]]; then
    ok "No other tracked line outside the tests names $old"
  else
    note "Lines outside the tests that still name $old. Review each:"
    while IFS= read -r line; do printf '  %s\n' "$line"; done <<< "$hits"
  fi
  note "$tests more in tests/ and worker/test/, test data that names its own version"
}

update_expected_keys() {
  local before after
  before=$(shasum -a 256 "$EXPECTED_KEYS" 2>/dev/null | cut -d' ' -f1)
  UPDATE_EXPECTED_KEYS=1 cargo test --quiet --test it \
    mirror_to_ods_fyi_test::mirror_dry_run_keys_match_expected_keys_json >/dev/null
  after=$(shasum -a 256 "$EXPECTED_KEYS" 2>/dev/null | cut -d' ' -f1)
  if [[ "$before" != "$after" ]]; then
    ok "$EXPECTED_KEYS regenerated"
  else
    ok "$EXPECTED_KEYS unchanged"
  fi
}

if [[ $# -eq 0 ]]; then
  printf '%-9s%-8s%s\n' "tool" "$(tool_version)" "$CARGO_TOML"
  printf '%-9s%-8s%s\n' "dataset" "$(dataset_version)" "$DATASET_RS (DATASET_VERSION)"
  exit 0
fi

[[ $# -eq 2 ]] || die "usage: scripts/bump-version.sh [tool|dataset] <x.y.z>"
KIND="$1"
NEW="$2"
is_semver "$NEW" || die "'$NEW' isn't a version. Use x.y.z, e.g. 0.2.0"

case "$KIND" in
  tool)
    OLD="$(tool_version)"
    [[ -n "$OLD" ]] || die "can't find the version in $CARGO_TOML"
    if [[ "$OLD" == "$NEW" ]]; then
      ok "tool is already $NEW"
    else
      perl -0pi -e 's/^version = "\Q'"$OLD"'\E"/version = "'"$NEW"'"/m' "$CARGO_TOML"
      [[ "$(tool_version)" == "$NEW" ]] || die "failed to set $CARGO_TOML to $NEW"
      cargo update --workspace --quiet
      ok "tool $OLD → $NEW  $CARGO_TOML, Cargo.lock"
    fi
    note "ods $NEW builds dataset $(dataset_version). Name it in CHANGELOG.md's entry for $NEW."
    if ! grep -qE "^## \[v?${NEW//./\\.}\]" CHANGELOG.md; then
      note "CHANGELOG.md has no '## [$NEW]' entry yet. scripts/release-tool.sh needs one."
    fi
    ;;
  dataset)
    OLD="$(dataset_version)"
    [[ -n "$OLD" ]] || die "can't find the version in $DATASET_RS"
    if [[ "$OLD" == "$NEW" ]]; then
      ok "dataset is already $NEW"
    else
      perl -0pi -e 's/^pub const DATASET_VERSION: &str = "\Q'"$OLD"'\E";/pub const DATASET_VERSION: &str = "'"$NEW"'";/m' "$DATASET_RS"
      [[ "$(dataset_version)" == "$NEW" ]] || die "failed to set $DATASET_RS to $NEW"
      UPDATE_SCHEMA=1 cargo test --quiet --test it schema_contract_test:: >/dev/null
      ok "dataset $OLD → $NEW  $DATASET_RS, $DATAPACKAGE_JSON"
      update_expected_keys
      note "Every Parquet file built from now on carries the new version, so every manifest digest changes."
      note "A dataset bump needs a tool bump at least as large: patch → patch, minor → minor, major → major. This is a $(bump_size "$OLD" "$NEW") bump; run \`scripts/bump-version.sh tool <x.y.z>\` to match it."
    fi
    ;;
  *)
    die "usage: scripts/bump-version.sh [tool|dataset] <x.y.z>"
    ;;
esac

[[ "$OLD" == "$NEW" ]] || report_leftovers "$OLD"
