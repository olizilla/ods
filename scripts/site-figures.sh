#!/usr/bin/env bash
# scripts/site-figures.sh [<YYYY-MM-DD> | <release-dir>]
#
# Reads a release with duckdb and ods, and writes site/src/data/release.json — every figure
# the ods.fyi home page shows. Prints the SQL it ran beside each value, so the page's numbers
# show their working the way `ods find --sql` does.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WORKSPACE="$REPO_ROOT/ods_data"
OUT="$REPO_ROOT/site/src/data/release.json"

die() { echo "✖ $*" >&2; exit 1; }

# The releases the workspace holds, by date, on one line
releases_held() {
  local dir found=""
  for dir in "$WORKSPACE"/releases/*/; do
    [ -d "$dir" ] && found="$found $(basename "$dir")"
  done
  echo "${found# }"
}

# Which release to read: none means the workspace's active release (`current`), YYYY-MM-DD means
# that release in the workspace, anything else is the path to a release directory.
if [ $# -gt 1 ]; then
  die "usage: scripts/site-figures.sh [<YYYY-MM-DD> | <release-dir>]"
fi
accepts="It takes:
    (no argument)  the active release, $WORKSPACE/current
    YYYY-MM-DD     a release in $WORKSPACE/releases/
    <path>         a release directory
  Workspace looked in: $WORKSPACE"

if [ $# -eq 0 ]; then
  [ -d "$WORKSPACE/current" ] || die "no active release: $WORKSPACE/current isn't a directory.
  $accepts"
  RELEASE_DIR="$(cd "$WORKSPACE/current" && pwd -P)"
elif [[ "$1" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
  [ -d "$WORKSPACE/releases/$1" ] || die "no release $1 in $WORKSPACE/releases/ (found: $(releases_held))
  $accepts"
  RELEASE_DIR="$(cd "$WORKSPACE/releases/$1" && pwd -P)"
else
  [ -d "$1" ] || die "'$1' is neither a release date nor a directory.
  $accepts"
  RELEASE_DIR="$(cd "$1" && pwd -P)"
fi

ORGS="$RELEASE_DIR/orgs.parquet"
ROLES="$RELEASE_DIR/roles.parquet"
RELATIONSHIPS="$RELEASE_DIR/relationships.parquet"
SUCCESSIONS="$RELEASE_DIR/successions.parquet"
for f in "$ORGS" "$ROLES" "$RELATIONSHIPS" "$SUCCESSIONS"; do
  if [ ! -f "$f" ]; then
    die "missing $f: $RELEASE_DIR isn't a built release"
  fi
done

# The release's provenance is the object every Parquet file carries under its `datapackage`
# key (docs/provenance.md): the page shows it as DuckDB reads it from orgs.parquet's footer.
PROVENANCE_SQL="SELECT decode(value) FROM parquet_kv_metadata('orgs.parquet') WHERE decode(key) = 'datapackage'"
echo "$PROVENANCE_SQL"
PROVENANCE="$(duckdb -noheader -list -c "SELECT decode(value) FROM parquet_kv_metadata('$ORGS') WHERE decode(key) = 'datapackage'")"
if [ -z "$PROVENANCE" ]; then
  die "$ORGS carries no provenance: build it from a pulled TRUD release"
fi

RELEASE_DATE="$(jq -r '.sources[0].version' <<<"$PROVENANCE")"
# `version` is <source release>_<dataset version>.
DATASET_VERSION="$(jq -r '.version | split("_")[1]' <<<"$PROVENANCE")"

echo "release: $RELEASE_DATE, dataset $DATASET_VERSION"
echo

# --- file sizes and record count -------------------------------------------------

orgs_size=$(stat -f%z "$ORGS" 2>/dev/null || stat -c%s "$ORGS")
roles_size=$(stat -f%z "$ROLES" 2>/dev/null || stat -c%s "$ROLES")
relationships_size=$(stat -f%z "$RELATIONSHIPS" 2>/dev/null || stat -c%s "$RELATIONSHIPS")
successions_size=$(stat -f%z "$SUCCESSIONS" 2>/dev/null || stat -c%s "$SUCCESSIONS")
total_size=$((orgs_size + roles_size + relationships_size + successions_size))

echo "SELECT count(*) FROM 'orgs.parquet'"
orgs_count=$(duckdb -csv -noheader -c "SELECT count(*) FROM '$ORGS';")
echo "  orgs.parquet: $orgs_count records, $((orgs_size)) bytes"
echo "SELECT count(*) FROM 'roles.parquet'"
roles_count=$(duckdb -csv -noheader -c "SELECT count(*) FROM '$ROLES';")
echo "  roles.parquet: $roles_count records, $roles_size bytes"
echo "SELECT count(*) FROM 'relationships.parquet'"
relationships_count=$(duckdb -csv -noheader -c "SELECT count(*) FROM '$RELATIONSHIPS';")
echo "  relationships.parquet: $relationships_count records, $relationships_size bytes"
echo "SELECT count(*) FROM 'successions.parquet'"
successions_count=$(duckdb -csv -noheader -c "SELECT count(*) FROM '$SUCCESSIONS';")
echo "  successions.parquet: $successions_count records, $successions_size bytes"
echo "  total: $total_size bytes across 4 tables"
echo

# --- GP practices: open orgs holding RO76, RO227 or RO315, by country -------------

gp_sql="SELECT country, count(*) FROM orgs
WHERE status = 'active' AND (legal_end IS NULL OR legal_end > publication_date)
  AND list_has_any(role_codes, ['RO76', 'RO227', 'RO315'])
GROUP BY country;"
echo "$gp_sql"
gp_json=$(duckdb -json -c "
CREATE VIEW orgs AS SELECT * FROM '$ORGS';
$gp_sql
")
echo "  $gp_json"
echo

# --- PCNs: open holders of RO272 ---------------------------------------------------

pcn_sql="SELECT count(*) FROM orgs
WHERE status = 'active' AND (legal_end IS NULL OR legal_end > publication_date)
  AND list_contains(role_codes, 'RO272');"
echo "$pcn_sql"
pcn_count=$(duckdb -csv -noheader -c "
CREATE VIEW orgs AS SELECT * FROM '$ORGS';
$pcn_sql
")
echo "  $pcn_count"
echo

# --- NHS trusts in England: RO197, country = ENGLAND, status = active -------------
# split open / moving to a successor / closing down with no successor

trusts_sql="SELECT
    status = 'active' AND (legal_end IS NULL OR legal_end > publication_date) AS open,
    len(successor_codes) = 0 AS no_successor,
    count(*)
  FROM orgs
  WHERE country = 'ENGLAND' AND list_contains(role_codes, 'RO197') AND status = 'active'
  GROUP BY 1, 2;"
echo "$trusts_sql"
trusts_json=$(duckdb -json -c "
CREATE VIEW orgs AS SELECT * FROM '$ORGS';
$trusts_sql
")
echo "  $trusts_json"

trust_open=$(echo "$trusts_json" | jq '[.[] | select(.open == true)][0]."count_star()" // 0')
trust_successor=$(echo "$trusts_json" | jq '[.[] | select(.open == false and .no_successor == false)][0]."count_star()" // 0')
trust_closing=$(echo "$trusts_json" | jq '[.[] | select(.open == false and .no_successor == true)][0]."count_star()" // 0')
trust_total=$((trust_open + trust_successor + trust_closing))
echo "  open $trust_open, moving to a successor $trust_successor, closing down with no successor $trust_closing (total marked active $trust_total)"
echo

# --- assemble release.json ---------------------------------------------------------

mkdir -p "$REPO_ROOT/site/src/data"

gp_total=$(echo "$gp_json" | jq '[.[]."count_star()"] | add')

jq -n \
  --arg date "$RELEASE_DATE" \
  --arg version "$DATASET_VERSION" \
  --argjson totalSizeBytes "$total_size" \
  --argjson orgsRecordCount "$orgs_count" \
  --argjson gpTotal "$gp_total" \
  --argjson gpByCountry "$(echo "$gp_json" | jq '[.[] | {country: .country, count: .["count_star()"]}] | sort_by(-.count)')" \
  --argjson pcnCount "$pcn_count" \
  --argjson trustsMarkedActive "$trust_total" \
  --argjson trustsOpen "$trust_open" \
  --argjson trustsMovingToSuccessor "$trust_successor" \
  --argjson trustsClosingNoSuccessor "$trust_closing" \
  --argjson orgsSizeBytes "$orgs_size" \
  --argjson rolesSizeBytes "$roles_size" \
  --argjson relationshipsSizeBytes "$relationships_size" \
  --argjson successionsSizeBytes "$successions_size" \
  --argjson rolesCount "$roles_count" \
  --argjson relationshipsCount "$relationships_count" \
  --argjson successionsCount "$successions_count" \
  --arg provenance "$PROVENANCE" \
  --arg provenanceSql "$PROVENANCE_SQL" \
  '{
    release: {
      date: $date,
      datasetVersion: $version,
      totalSizeBytes: $totalSizeBytes,
      orgsRecordCount: $orgsRecordCount
    },
    gpPractices: {
      total: $gpTotal,
      byCountry: $gpByCountry
    },
    pcns: {
      total: $pcnCount
    },
    trusts: {
      markedActive: $trustsMarkedActive,
      open: $trustsOpen,
      movingToSuccessor: $trustsMovingToSuccessor,
      closingNoSuccessor: $trustsClosingNoSuccessor
    },
    files: [
      { name: "orgs.parquet", records: $orgsRecordCount, sizeBytes: $orgsSizeBytes },
      { name: "relationships.parquet", records: $relationshipsCount, sizeBytes: $relationshipsSizeBytes },
      { name: "roles.parquet", records: $rolesCount, sizeBytes: $rolesSizeBytes },
      { name: "successions.parquet", records: $successionsCount, sizeBytes: $successionsSizeBytes }
    ],
    provenance: $provenance,
    provenanceSql: $provenanceSql
  }' > "$OUT"

echo "wrote $OUT"
