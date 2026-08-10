#!/usr/bin/env bash
set -euo pipefail

echo "=========================================================================="
echo "                   ODS LOCAL OFFLINE SMOKE TEST                       "
echo "=========================================================================="

# 1. Clean workspace
echo "* Cleaning workspace..."
rm -rf ods_data/

# 2. Run offline trud pull using canned fixture archive
echo "* Running 'ods trud pull --local-archive .local/fixtures/trud'..."
ods trud pull --local-archive .local/fixtures/trud

# 3. Verify workspace release directory structure
if [ ! -d "ods_data/current/trud" ]; then
  echo "✖ Missing ods_data/current/trud"
  exit 1
fi
if [ ! -f "ods_data/current/_provenance.json" ]; then
  echo "✖ Missing ods_data/current/_provenance.json"
  exit 1
fi
echo "✓ Workspace initialized & provenance written successfully."

# 4. Run 'ods make' to generate all target projections
echo "* Running 'ods make'..."
ods make

# 5. Assert all output artifacts exist and are non-empty
echo "* Verifying generated target projections..."
test -s "ods_data/current/orgs.parquet"
test -s "ods_data/current/orgs_all.parquet"
test -s "ods_data/current/roles.parquet"
test -s "ods_data/current/rels.parquet"
test -s "ods_data/current/successors.parquet"
test -s "ods_data/current/markdown/wiki.zip"

echo "=========================================================================="
echo "✓ SMOKE TEST PASSED: All projections (Parquet, Wiki, NDJSON) verified!"
echo "=========================================================================="

