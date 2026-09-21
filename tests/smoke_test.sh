#!/usr/bin/env bash
set -euo pipefail

echo "=========================================================================="
echo "                   ODS LOCAL OFFLINE SMOKE TEST                       "
echo "=========================================================================="

# 1. Clean workspace
echo "* Cleaning workspace..."
rm -rf ods_data/
TMP_FIXTURE_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_FIXTURE_DIR"' EXIT
zip -j -q "$TMP_FIXTURE_DIR/hscorgrefdataxml_data_7.0.0_20260731000001.zip" tests/fixtures/mock_hscorgrefdata.xml

# 2. Run 'ods make parquet' to generate all target projections from mock fixture
echo "* Running 'ods make parquet' from committed mock fixture..."
cargo run -- make parquet --input "$TMP_FIXTURE_DIR/hscorgrefdataxml_data_7.0.0_20260731000001.zip" --output ods_data/current

# 3. Assert all output artifacts exist and are non-empty
echo "* Verifying generated target projections..."
test -s "ods_data/current/orgs.parquet"
test -s "ods_data/current/roles.parquet"
test -s "ods_data/current/relationships.parquet"
test -s "ods_data/current/successions.parquet"

echo "=========================================================================="
echo "✓ SMOKE TEST PASSED: All projections (Parquet) verified!"
echo "=========================================================================="

