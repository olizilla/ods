#!/usr/bin/env bash
set -euo pipefail

# Single source of truth: Run the deterministic mirror_to_ods_fyi_test in update mode
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "Regenerating worker/test/fixtures/expected-keys.json via mirror_to_ods_fyi_test..."
UPDATE_EXPECTED_KEYS=1 cargo test --test it -- mirror_to_ods_fyi_test::mirror_dry_run_keys_match_expected_keys_json --exact --nocapture
echo "✓ Successfully regenerated worker/test/fixtures/expected-keys.json"
