# ods - NHS Organisation Data CLI

A Rust CLI to make NHS Organisation Data Service (ODS) records easy to query and use by transforming raw TRUD ODS XML into modern, open formats.

## Key Features

- **Easy to query out of the box**: `ods find` gives you fast, local-first search across all NHS orgs and sites. Parent, PCN, ICB, Trust, and Successor relationships are all pre-calculated.
- **Open formats**: Derived Parquet tables can be queried remotely using `duckdb`, Python Pandas, or Polars without installing `ods` at all.
- **Reproducible projections**: Built directly from official NHS TRUD XML data with full provenance tracking and SHA-256 hash checks. You can build byte-for-byte identical projections yourself anytime with `ods make`.

A sister project to `sct`, the SNOMED CT CLI. `ods` aims to provide a stable, academically citable and easier to interpret source for NHS Org structure info.

The CLI serves three distinct users:

| Goal | Need | Commands |
|---|---|---|
| **Query** — "I just want to search NHS orgs" | Zero-setup search, citable provenance | `find`, `pull`, `cite` |
| **Verify** — "I want to build from source and check hashes" | TRUD API access, reproducible builds | `trud pull`, `make`, `trud audit`, `trud verify` |
| **Publish** — "I produce the monthly release artifacts" | Full pipeline, changelogs, CI integration | `trud pull`, `make`, `trud diff`, `trud audit` |

## Quick Example with DuckDB

Query hosted or local Parquet files directly:

```bash
# Find active GP Practices
duckdb -c "
SELECT name, ods_code, postcode, telephone, pcn, icb
FROM 'ods_data/current/orgs.parquet'
WHERE role IN ('Prescribing Cost Centre', 'General Practice')
ORDER BY postcode;
"
```

```sql
-- Find active NHS entities in Sedbergh
SELECT ods_code, name, role, record_class, postcode
FROM 'ods_data/current/orgs.parquet'
WHERE town = 'SEDBERGH';
```

## Workspace Directory (`ods_data`)

The `ods` CLI manages a local workspace directory (`ods_data`) containing release snapshots:

```text
ods_data/
├── README.md                           # Workspace schema documentation
├── current -> releases/2026-07-31/     # Active release pin (symlink)
└── releases/
    └── 2026-07-31/                    # Release directory
        ├── _provenance.json           # Source TRUD metadata & SHA-256 derived hashes
        ├── orgs.parquet               # Active orgs & sites (Primary analytical table)
        ├── orgs_all.parquet           # All orgs (active + inactive)
        ├── rels.parquet               # Target relationship links (ICB, Trust, PCN)
        ├── roles.parquet              # Primary & secondary roles
        ├── SHA256SUMS                 # Flat verification manifest for _provenance.json and *.parquet
        ├── successors.parquet         # Entity successor chains
        ├── markdown/                  # Open Knowledge Format (OKF) wiki archive
        │   └── wiki.zip
        └── trud/                      # TRUD zip archives (gitignored)
```

`ods pull` (WIP) will fetch the parquet files and provenance.json from the latest github release, and verify the data integrity. The parquet files are reproducibly derived from the official NHS TRUD ODS XML data, and will be published automatically from CI each month when a new XML source file is published.

See [docs/ods_data.md](file:///Users/oli/Code/olizilla/ods/docs/ods_data.md) for full workspace specification details.

## Commands

```
Usage: ods <COMMAND>

Commands:
  find  Search NHS organisations and sites
  pull  Download pre-built dataset releases
  cite  Show provenance metadata and academic citation
  trud  Build from official NHS source data (requires TRUD API key)
  make  Compile TRUD XML into Parquet tables and Markdown
  help  Print this message or the help of the given subcommand(s)
```


### find

Searches for organisations or sites in the Parquet tables by name, ODS code, or postcode. Automatically initialises `./ods_data/` with pre-compiled Parquet files if no workspace exists.

```bash
ods find "Royal Free"
ods find --role "General Practice" "SW9" --format csv
```

### pull

Pulls pre-compiled ODS Parquet release datasets into `./ods_data/`, lists available release versions, or switches active release pins.

```bash
ods pull                  # Pull latest release dataset
ods pull --list           # List all remote and local release versions (-l)
ods pull 2026-06-22       # Switch workspace pin or pull specific release
```

### cite

Shows academic citation blocks, active workspace release status, entity counts, and SHA-256 hashes.

```bash
ods cite
```

### trud

Subcommands for working with the NHS TRUD REST API (`pull`, `diff`, `audit`, `verify`).

```bash
NHS_TRUD_API_KEY="<key>" ods trud pull
ods trud diff --old 2026-06-22 --new 2026-07-31
ods trud audit
ods trud verify
```

### make

Generates target projections (Parquet tables + OKF Markdown wiki) from raw TRUD XML data.

```bash
ods make                  # Generate target projections (Parquet + Markdown)
ods make parquet          # Generate Parquet tables from TRUD XML
ods make md               # Generate OKF Markdown wiki archive from Parquet
```

## Getting Started

Building from source requires Rust:

```bash
cargo build --release
```

Run tests:

```bash
cargo test
```
