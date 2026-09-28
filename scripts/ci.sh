#!/usr/bin/env bash
# Runs the steps CI runs, times each one, and saves a summary so runs can be compared.
#
#   scripts/ci.sh                          run every step
#   scripts/ci.sh --release-data           also run the #[ignore]d tests against ods_data/releases/2026-08-28
#   scripts/ci.sh --compare OLD.tsv        run, then compare with an earlier summary
#   scripts/ci.sh --compare OLD.tsv NEW.tsv   compare two summaries without running
#
# Summaries and step logs go to target/ci-runs/. A local run (not in CI, and not
# itself a --compare) prints its own comparison against the most recent earlier
# summary in that directory, so plain `scripts/ci.sh` output already shows the
# diff. Compare runs from the same machine: a laptop and a CI runner differ
# several times over, and so do cold and warm caches.
set -uo pipefail

cd "$(dirname "$0")/.." || exit

release_data=0
compare_old=""
compare_new=""
while [ $# -gt 0 ]; do
  case "$1" in
    --release-data) release_data=1; shift ;;
    --compare)
      compare_old=${2:?--compare needs a summary file}; shift 2
      if [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; then compare_new=$1; shift; fi
      ;;
    -h|--help) sed -n '2,10p' "$0"; exit 0 ;;
    *) echo "✖ Unknown argument: $1" >&2; exit 2 ;;
  esac
done

compare() {
  awk -F'\t' '
    FNR == 1 { file++ }
    /^#/ || $1 == "step" { next }
    file == 1 { before[$1] = $3; order[++n] = $1 }
    file == 2 { after[$1] = $3; if (!($1 in before)) order[++n] = $1 }
    END {
      printf "%-20s %8s %8s %8s\n", "step", "before", "after", "change"
      for (i = 1; i <= n; i++) {
        s = order[i]; b = before[s]; a = after[s]
        if (b == "" || a == "") printf "%-20s %8s %8s %8s\n", s, (b == "" ? "-" : b), (a == "" ? "-" : a), "-"
        else printf "%-20s %8.1f %8.1f %+8.1f\n", s, b, a, a - b
      }
    }' "$1" "$2"
}

if [ -n "$compare_new" ]; then
  compare "$compare_old" "$compare_new"
  exit 0
fi

now() { perl -MTime::HiRes=time -e 'printf "%.2f", time'; }
has() { command -v "$1" >/dev/null 2>&1; }

sha=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
dirty=""
[ -n "$(git status --porcelain --untracked-files=no 2>/dev/null)" ] && dirty="-dirty"
run_dir="target/ci-runs/$(date +%Y-%m-%dT%H-%M-%S)_${sha}${dirty}"
summary="$run_dir.tsv"
mkdir -p "$run_dir"

in_ci=${GITHUB_ACTIONS:-false}
names=(); statuses=(); seconds=(); details=()
failed=0

add_row() { # name status seconds detail
  names+=("$1"); statuses+=("$2"); seconds+=("$3"); details+=("$4")
  printf '%-20s %-8s %8s  %s\n' "$1" "$2" "$3" "$4"
}

# run NAME FUNCTION: runs the step, logging to $LOG; sets STATUS and SECS.
run() {
  STEP=$1
  LOG="$run_dir/$(echo "$1" | tr ' /' '--').log"
  [ "$in_ci" = true ] && echo "::group::$STEP"
  local start; start=$(now)
  if [ "$in_ci" = true ]; then
    "$2" 2>&1 | tee "$LOG"; STATUS=${PIPESTATUS[0]}
  else
    "$2" >"$LOG" 2>&1; STATUS=$?
  fi
  SECS=$(awk -v a="$start" -v b="$(now)" 'BEGIN { printf "%.1f", b - a }')
  [ "$in_ci" = true ] && echo "::endgroup::"
  return 0
}

# record DETAIL: adds the row for the step `run` just finished.
record() {
  local status=ok
  if [ "$STATUS" -ne 0 ]; then status=FAILED; failed=1; fi
  add_row "$STEP" "$status" "$SECS" "$1"
  if [ "$status" = FAILED ] && [ "$in_ci" != true ]; then
    echo "   last 40 lines of $LOG:"
    tail -n 40 "$LOG" | sed 's/^/   │ /'
  fi
}

skip() { add_row "$1" skipped "" "$2"; }

# ---------------------------------------------------------------------------
# Steps

compile() { cargo test --no-fail-fast --no-run; }

# `-D warnings` turns every clippy warning into a hard error, so a clean run exits 0 and a
# dirty one fails the build; the count comes from the diagnostic lines it printed, not the
# exit code, since cargo's own "could not compile ... due to N previous errors" summary line
# repeats once per target (lib, lib test, each integration test binary) and would overcount.
clippy_check() { cargo clippy --all-targets -- -D warnings; }

rust_tests() {
  local leaks
  leaks=$(grep -rn 'CARGO_BIN_EXE_ods' tests/ | grep -v '^tests/it/common/mod\.rs:' || true)
  if [ -n "$leaks" ]; then
    echo "CARGO_BIN_EXE_ods used outside tests/it/common/mod.rs:" >&2
    echo "$leaks" >&2
    return 1
  fi

  local hostile_home hostile_cache orig_home orig_rustup orig_cargo
  hostile_home=$(mktemp -d)
  hostile_cache=$(mktemp -d)
  orig_home="$HOME"
  orig_rustup="${RUSTUP_HOME:-$orig_home/.rustup}"
  orig_cargo="${CARGO_HOME:-$orig_home/.cargo}"

  mkdir -p "$hostile_home/ods_data"
  echo "{}" > "$hostile_home/ods_data/_releases.json"

  # cargo runs test binaries with the package root as their working directory
  # regardless of where `cargo test` itself is invoked from (verified: running from
  # a fresh directory with --manifest-path still leaves stray writes in the repo
  # root, not there), so a temporary invocation directory can't catch a test that
  # writes without saying where. Watch the repo root itself instead: any file or
  # directory `git status` didn't see before the run and does after — tracked or
  # (like ods_data/) gitignored — is left behind by a test.
  local before after stray status
  before=$(git status --porcelain --ignored=matching --untracked-files=all)
  if [ "$release_data" = 1 ]; then
    RUSTUP_HOME="$orig_rustup" \
    CARGO_HOME="$orig_cargo" \
    COLUMNS=40 \
    ODS_RELEASE_INDEX_URL="http://127.0.0.1:9/unreachable.json" \
    TRUD_API_KEY="not-a-key" \
    ODS_CACHE_DIR="$hostile_cache" \
    HOME="$hostile_home" \
    cargo test --no-fail-fast -- --include-ignored
  else
    RUSTUP_HOME="$orig_rustup" \
    CARGO_HOME="$orig_cargo" \
    COLUMNS=40 \
    ODS_RELEASE_INDEX_URL="http://127.0.0.1:9/unreachable.json" \
    TRUD_API_KEY="not-a-key" \
    ODS_CACHE_DIR="$hostile_cache" \
    HOME="$hostile_home" \
    cargo test --no-fail-fast
  fi
  status=$?
  rm -rf "$hostile_home" "$hostile_cache"

  after=$(git status --porcelain --ignored=matching --untracked-files=all)
  stray=$(comm -13 <(echo "$before" | sort) <(echo "$after" | sort))
  if [ -n "$stray" ]; then
    echo "left behind in the repo root: $(echo "$stray" | tr '\n' ' ')"
    status=1
  fi
  return $status
}

smoke() {
  local tmp status
  tmp=$(mktemp -d)
  local zip="$tmp/hscorgrefdataxml_data_7.0.0_20260731000001.zip"
  local ws="$tmp/ods_data"
  local index="$tmp/candidate_index.json"
  zip -j -q "$zip" tests/fixtures/mock_hscorgrefdata.xml && {
    local zip_sha; zip_sha=$(shasum -a 256 "$zip" | cut -d' ' -f1 | tr '[:lower:]' '[:upper:]')
    local zip_size; zip_size=$(wc -c < "$zip" | tr -d ' ')
    cat <<EOF > "$index"
{
  "\$schema": "https://ods.fyi/schema/releases.v1.json",
  "trud_signing_key_fingerprints": [
    "0000000000000000000000000000000000000000"
  ],
  "mirrors": [],
  "releases": [
    {
      "trud_release_date": "2026-07-31",
      "trud_release_sha256": "$zip_sha",
      "trud_release_filesize_bytes": $zip_size,
      "datasets": []
    }
  ]
}
EOF
  } &&
    ./target/debug/ods make --input "$zip" --output "$ws/releases/2026-07-31" --index "$index" &&
    ./target/debug/ods trud audit --input "$zip" --workspace "$ws" --full &&
    ./target/debug/ods make oci --input "$ws/releases/2026-07-31" &&
    ./target/debug/ods make oci --input "$ws/releases/2026-07-31" --check
  status=$?
  rm -rf "$tmp"
  return $status
}

# Rebuilds the newest dataset published at this build's DATASET_VERSION from its kept archive
# (nhs-ods-xml), and compares the manifest digest with the one recorded for it, so a change to the
# bytes without a dataset version bump fails here instead of at the pull request that caused it
# (docs/tests.md D5). CI_REPRODUCE_INDEX reads another index in place of data/releases.json — this
# brief's acceptance points it at a candidate.json from a rehearsal run. CI_REPRODUCE_ARCHIVE names a
# local directory holding the release's four NHS files (a release's trud/, say) to build from in
# place of pulling nhs-ods-xml, for a rehearsal with no registry credential.
#
# Prints one "RESULT:<skip|ok|fail>:<detail>" line; the caller below turns that into the row.
# Skips, naming why, without oras or a working registry credential (a laptop gh token without
# read:packages, or — until the maintainer sets up CI's own credential — any run at all); the pull
# request that changed the bytes is still caught once this runs on main after merge. `ods make` is
# measured with the debug build already compiled by the "rust compile" step; it took under a
# minute on a full release when this was written, so there's no --release fallback (yet) — if a
# future release makes that too slow, build --release for this step alone and report both times.
reproduce() {
  local index="${CI_REPRODUCE_INDEX:-data/releases.json}"
  local dataset_version
  dataset_version=$(sed -n 's/^pub const DATASET_VERSION: &str = "\([^"]*\)";/\1/p' src/datapackage.rs | head -1)

  local date row_digest
  date=$(jq -r --arg v "$dataset_version" '
    [.releases[] | select(.datasets[]? | .dataset_version == $v)] | sort_by(.trud_release_date) | last | .trud_release_date // empty
  ' "$index" 2>/dev/null)
  if [ -z "$date" ]; then
    echo "RESULT:skip:no release published at dataset $dataset_version yet"
    return 0
  fi
  row_digest=$(jq -r --arg v "$dataset_version" --arg d "$date" '
    .releases[] | select(.trud_release_date == $d) | .datasets[] | select(.dataset_version == $v) | .manifest_digest
  ' "$index")

  local work trud pull_err
  work=$(mktemp -d)
  if [ -n "${CI_REPRODUCE_ARCHIVE:-}" ]; then
    trud="$CI_REPRODUCE_ARCHIVE"
  else
    trud="$work/trud"
    pull_archive "$date" "$trud"
    case $? in
      0) ;;
      1) rm -rf "$work"; return 0 ;;
      *) rm -rf "$work"; return 1 ;;
    esac
  fi

  if ! scripts/verify-trud-bundle.sh "$trud"; then
    rm -rf "$work"
    return 1
  fi

  # The manifest digest comes from `ods make oci`'s own report, not from reading oci/: that holds
  # the empty config blob as a real file too.
  local zip start elapsed rebuilt_digest
  zip=$(find "$trud" -maxdepth 1 -name '*.zip' | head -n 1)
  start=$(now)
  if ! ./target/debug/ods make -i "$zip" -o "$work/rebuilt" --index "$index" >/dev/null \
    || ! rebuilt_digest=$(./target/debug/ods make oci -i "$work/rebuilt" --format json | jq -r .manifest_digest); then
    rm -rf "$work"
    return 1
  fi
  elapsed=$(awk -v a="$start" -v b="$(now)" 'BEGIN { printf "%.1f", b - a }')
  rm -rf "$work"

  if [ "$rebuilt_digest" = "$row_digest" ]; then
    echo "RESULT:ok:$date rebuilt to $rebuilt_digest (debug build, ${elapsed}s)"
    return 0
  fi

  echo "✖ The bytes changed without a dataset version bump. Bump the dataset version, or find the unintended change." >&2
  echo "  the index  $row_digest" >&2
  echo "  rebuilt    $rebuilt_digest" >&2
  echo "RESULT:fail:$date rebuilt to $rebuilt_digest, the index has $row_digest"
  return 1
}

# pull_archive DATE DIR: pulls nhs-ods-xml:DATE into DIR and returns 0. Returns 1 for a skip (no
# credential, or the registry refused), having printed the RESULT:skip line, and 2 for any other
# failure, having printed why.
pull_archive() {
  local date="$1" dir="$2" pull_err
  # This step needs no credential of its own: it logs in only if one is already sitting there,
  # and a missing or under-scoped one turns into a skip when the pull below is refused.
  if [ "${GITHUB_ACTIONS:-false}" = "true" ]; then
    if [ -n "${GITHUB_TOKEN:-}" ] && [ -n "${GITHUB_ACTOR:-}" ]; then
      echo "$GITHUB_TOKEN" | oras login ghcr.io -u "$GITHUB_ACTOR" --password-stdin >/dev/null 2>&1
    fi
  elif has gh && gh auth status 2>&1 | grep -qE 'read:packages|write:packages'; then
    gh auth token 2>/dev/null | oras login ghcr.io -u "$(gh api user -q .login 2>/dev/null)" --password-stdin >/dev/null 2>&1
  else
    echo "RESULT:skip:gh auth token without read:packages"
    return 1
  fi

  pull_err="$dir.pull.err"
  if ! oras pull "ghcr.io/olizilla/nhs-ods-xml:$date" -o "$dir" >/dev/null 2>"$pull_err"; then
    if grep -qiE 'unauthorized|denied|forbidden|401|403' "$pull_err"; then
      echo "RESULT:skip:registry refused the pull for $date"
      return 1
    fi
    echo "reproduce: oras pull failed for $date:" >&2
    cat "$pull_err" >&2
    return 2
  fi
}

lock_sha() { shasum -a 256 worker/package-lock.json | cut -d' ' -f1; }

worker_install() {
  if [ -f worker/node_modules/.ci-lock-sha ] && [ "$(cat worker/node_modules/.ci-lock-sha)" = "$(lock_sha)" ]; then
    echo "worker/node_modules already matches package-lock.json"
    return 0
  fi
  npm ci --prefix worker && lock_sha > worker/node_modules/.ci-lock-sha
}

# The worker imports worker/src/site-root-files.json, which the site build generates.
worker_typecheck() { (cd worker && npx tsc --noEmit); }

site_lock_sha() { shasum -a 256 site/package-lock.json | cut -d' ' -f1; }

site_install() {
  if [ -f site/node_modules/.ci-lock-sha ] && [ "$(cat site/node_modules/.ci-lock-sha)" = "$(site_lock_sha)" ]; then
    echo "site/node_modules already matches package-lock.json"
    return 0
  fi
  npm ci --prefix site && site_lock_sha > site/node_modules/.ci-lock-sha
}

site_build() { npm run build --prefix site; }

# worker tests' [assets] directory is site/dist, so the site must build first.
worker_tests() { npm test --prefix worker; }

has_conformance() {
  has conformance && return 0
  has go && [ -x "$(go env GOPATH)/bin/conformance" ] && return 0
  [ -x "$HOME/go/bin/conformance" ]
}

# ---------------------------------------------------------------------------

printf 'ods ci  %s%s  %s  %s\n' "$sha" "$dirty" "$(date '+%Y-%m-%d %H:%M')" "$(uname -sm)"
printf '%-20s %-8s %8s  %s\n' step status seconds detail

deps=$(cargo metadata --format-version 1 2>/dev/null | jq '.packages | length')

run "rust compile" compile
if [ "$STATUS" -eq 0 ]; then
  crates=$(grep -c '^ *Compiling ' "$LOG")
  case "$crates" in
    0) detail="nothing to compile" ;;
    1) detail="compiled 1 crate" ;;
    *) detail="compiled $crates crates" ;;
  esac
else
  detail=""
fi
[ -n "$deps" ] && detail="$detail${detail:+ · }$deps deps"
record "$detail"
compiled=$STATUS

if [ "$compiled" -ne 0 ]; then
  skip "clippy" "compile failed"
  skip "rust tests" "compile failed"
  skip "reproduce" "compile failed"
  skip "smoke make/audit" "compile failed"
else
  run "clippy" clippy_check
  if [ "$STATUS" -eq 0 ]; then
    record "ok"
  else
    warnings=$(awk '/^error: could not compile/{c++} /^error: /{t++} END{print t-c}' "$LOG")
    record "$warnings warnings"
  fi

  if [ "$release_data" = 1 ] && [ -z "${TRUD_XML_PATH:-}" ]; then
    TRUD_XML_PATH=$(ls "$PWD"/ods_data/releases/2026-08-28/trud/*.zip 2>/dev/null | head -n 1)
    export TRUD_XML_PATH
  fi
  run "rust tests" rust_tests
  detail=$(grep -h 'test result:' "$LOG" | awk '
    { p += $4; f += $6; i += $8; t = $NF; sub(/s$/, "", t); s += t }
    END { printf "%d passed, %d failed, %d ignored, %.1f s inside tests", p, f, i, s }')
  has duckdb || detail="$detail; duckdb not installed, so its checks skip"
  [ "$release_data" = 1 ] && detail="$detail; with ignored tests"
  stray_detail=$(grep -h '^left behind in the repo root' "$LOG")
  [ -n "$stray_detail" ] && detail="$detail; $stray_detail"
  record "$detail"

  if has oras; then
    run "reproduce" reproduce
    result_line=$(grep -h '^RESULT:' "$LOG" | tail -n 1)
    detail=${result_line#RESULT:*:}
    case "$result_line" in
      RESULT:skip:*) add_row "reproduce" skipped "$SECS" "$detail" ;;
      *) record "$detail" ;;
    esac
  else
    skip "reproduce" "oras not installed"
  fi

  if has zip; then
    run "smoke make/audit" smoke
    record ""
  else
    skip "smoke make/audit" "zip not installed"
  fi
fi

if ! has npm; then
  skip "worker install" "npm not installed"
  skip "site install" "npm not installed"
  skip "site build" "npm not installed"
  skip "worker typecheck" "npm not installed"
  skip "worker tests" "npm not installed"
else
  run "worker install" worker_install
  if grep -q 'already matches' "$LOG"; then record "lockfile unchanged"; else record ""; fi
  installed=$STATUS

  if [ "$installed" -ne 0 ]; then
    skip "site install" "install failed"
    skip "site build" "install failed"
    skip "worker typecheck" "install failed"
    skip "worker tests" "install failed"
  else
    run "site install" site_install
    if grep -q 'already matches' "$LOG"; then record "lockfile unchanged"; else record ""; fi
    site_installed=$STATUS

    if [ "$site_installed" -ne 0 ]; then
      skip "site build" "install failed"
      skip "worker typecheck" "site install failed, so there is no worker/src/site-root-files.json"
      skip "worker tests" "site install failed"
    else
      run "site build" site_build
      record ""
      site_built=$STATUS

      if [ "$site_built" -ne 0 ]; then
        skip "worker typecheck" "site build failed, so there is no worker/src/site-root-files.json"
      else
        run "worker typecheck" worker_typecheck
        record ""
      fi

      if [ "$compiled" -ne 0 ]; then
        skip "worker tests" "compile failed, so there is no target/debug/ods"
      elif [ "$site_built" -ne 0 ]; then
        skip "worker tests" "site build failed, so there is no site/dist"
      else
        run "worker tests" worker_tests
        detail=$(sed 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -E '^ *Tests ' | tail -n 1 | sed 's/^ *Tests *//')
        has_conformance || detail="$detail; OCI conformance runner not installed, so its test skips"
        record "$detail"
      fi
    fi
  fi
fi

total=$(printf '%s\n' "${seconds[@]}" | awk '{ s += $1 } END { printf "%.1f", s }')
total_status=ok
[ "$failed" -ne 0 ] && total_status=FAILED
add_row total "$total_status" "$total" ""

{
  printf '# ods ci\t%s%s\t%s\t%s\t%s\n' "$sha" "$dirty" "$(date '+%Y-%m-%d %H:%M')" "$(uname -sm)" "$(rustc -V 2>/dev/null)"
  printf 'step\tstatus\tseconds\tdetail\n'
  for i in "${!names[@]}"; do
    printf '%s\t%s\t%s\t%s\n' "${names[$i]}" "${statuses[$i]}" "${seconds[$i]}" "${details[$i]}"
  done
} > "$summary"
echo "saved $summary"

if [ "$in_ci" != true ] && [ -z "$compare_old" ]; then
  # The newest earlier summary: globs expand sorted, and summaries are named by timestamp
  prev=""
  for f in target/ci-runs/*.tsv; do
    [ -e "$f" ] && [ "$f" != "$summary" ] && prev="$f"
  done
  if [ -n "$prev" ]; then
    echo
    echo "compared with $prev:"
    compare "$prev" "$summary"
  fi
fi

if [ "$in_ci" = true ] && [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "| step | status | seconds | detail |"
    echo "| :--- | :--- | ---: | :--- |"
    for i in "${!names[@]}"; do
      echo "| ${names[$i]} | ${statuses[$i]} | ${seconds[$i]} | ${details[$i]} |"
    done
  } >> "$GITHUB_STEP_SUMMARY"
fi

if [ -n "$compare_old" ]; then
  echo
  compare "$compare_old" "$summary"
fi

exit "$failed"
