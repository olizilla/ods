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

rust_tests() {
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
    cargo test --no-fail-fast -- --include-ignored
  else
    cargo test --no-fail-fast
  fi
  status=$?
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
  zip -j -q "$zip" tests/fixtures/mock_hscorgrefdata.xml &&
    ./target/debug/ods make --input "$zip" --output "$ws/releases/2026-07-31" &&
    ./target/debug/ods trud audit --input "$zip" --workspace "$ws" --full &&
    ./target/debug/ods make oci --input "$ws/releases/2026-07-31" &&
    ./target/debug/ods make oci --input "$ws/releases/2026-07-31" --check
  status=$?
  rm -rf "$tmp"
  return $status
}

lock_sha() { shasum -a 256 worker/package-lock.json | cut -d' ' -f1; }

worker_install() {
  if [ -f worker/node_modules/.ci-lock-sha ] && [ "$(cat worker/node_modules/.ci-lock-sha)" = "$(lock_sha)" ]; then
    echo "worker/node_modules already matches package-lock.json"
    return 0
  fi
  npm ci --prefix worker && lock_sha > worker/node_modules/.ci-lock-sha
}

worker_typecheck() { (cd worker && npx tsc --noEmit); }

worker_tests() { npm test --prefix worker; }

has_conformance() {
  has conformance && return 0
  has go && [ -x "$(go env GOPATH)/bin/conformance" ] && return 0
  [ -x "$HOME/go/bin/conformance" ]
}

# ---------------------------------------------------------------------------

printf 'ods ci  %s%s  %s  %s\n' "$sha" "$dirty" "$(date '+%Y-%m-%d %H:%M')" "$(uname -sm)"
printf '%-20s %-8s %8s  %s\n' step status seconds detail

run "rust compile" compile
if [ "$STATUS" -eq 0 ]; then
  crates=$(grep -c '^ *Compiling ' "$LOG")
  case "$crates" in
    0) record "nothing to compile" ;;
    1) record "compiled 1 crate" ;;
    *) record "compiled $crates crates" ;;
  esac
else
  record ""
fi
compiled=$STATUS

if [ "$compiled" -ne 0 ]; then
  skip "rust tests" "compile failed"
  skip "smoke make/audit" "compile failed"
else
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

  if has zip; then
    run "smoke make/audit" smoke
    record ""
  else
    skip "smoke make/audit" "zip not installed"
  fi
fi

if ! has npm; then
  skip "worker install" "npm not installed"
  skip "worker typecheck" "npm not installed"
  skip "worker tests" "npm not installed"
else
  run "worker install" worker_install
  if grep -q 'already matches' "$LOG"; then record "lockfile unchanged"; else record ""; fi
  installed=$STATUS

  if [ "$installed" -ne 0 ]; then
    skip "worker typecheck" "install failed"
    skip "worker tests" "install failed"
  else
    run "worker typecheck" worker_typecheck
    record ""
    if [ "$compiled" -ne 0 ]; then
      skip "worker tests" "compile failed, so there is no target/debug/ods"
    else
      run "worker tests" worker_tests
      detail=$(sed 's/\x1b\[[0-9;]*m//g' "$LOG" | grep -E '^ *Tests ' | tail -n 1 | sed 's/^ *Tests *//')
      has_conformance || detail="$detail; OCI conformance runner not installed, so its test skips"
      record "$detail"
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
  prev=$(ls -1 target/ci-runs/*.tsv 2>/dev/null | grep -v -x -F "$summary" | sort | tail -n 1)
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
