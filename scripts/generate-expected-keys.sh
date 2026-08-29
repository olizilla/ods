#!/usr/bin/env bash
set -euo pipefail

# Single source of truth: Run the deterministic make_release_test in update mode
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

echo "Regenerating worker/test/fixtures/expected-keys.json via make_release_test..."
UPDATE_EXPECTED_KEYS=1 cargo test --test make_release_test -- test_make_release_creates_dist_staging_tree_with_real_files --exact --nocapture
echo "✓ Successfully regenerated worker/test/fixtures/expected-keys.json"
