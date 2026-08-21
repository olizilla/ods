# `ods` - NHS Organisation Data Service CLI

A Rust CLI to make NHS Organisation Data Service (ODS) records easy to query and use by transforming raw TRUD ODS XML into modern, open formats.

The `ods` project lets use and explore the data in in multiple ways:

| I want to...                  | Feature                | Commands |
|-------------------------------|------------------------|----------|
| _**Query** the data with SQL_ | Hosted Parquet files   | `SELECT * FROM https://ods.fyi/orgs.parquet` |
| _**Find** NHS orgs_           | Local-first search     | `ods pull` `ods find` |
| _**Cite** the data_           | Citation guide         | `ods cite` |
| _**Verify** the proveance_    | Reproducible builds    | `ods trud pull` `ods make` `ods trud audit` |
| _What changed this month?_    | Diff trud ods releases | `ods trud diff` |

- **Open formats**: Derived Parquet tables can be queried remotely using `duckdb`, or Python without installing `ods` at all.
- **Easy to query out of the box**: `ods find` gives you fast, local-first search across all NHS orgs and sites. Parent, PCN, ICB, Trust, and Successor relationships are all pre-calculated.
- **Reproducible projections**: Built directly from official NHS TRUD XML data with full provenance tracking and SHA-256 hash checks. You can build byte-for-byte identical projections yourself anytime with `ods make`.

A sister project to `sct`, the SNOMED CT CLI. `ods` aims to provide a stable, academically citable and easier to interpret source for NHS Org structure info.

> **NOTE** this an independent project not commissioned by the NHS. The goal is to provide reliable and useful projections of NHS data, to aid research and to demonstrate the value of open formats and good UX.

## Query the data

Use `duckdb` to query the hosted parquet files. 

Use `ods pull` to fetch the dataset to your computer. 

Use `ods find` to explore the data without SQL.

### Find all active GP practices, and their parent ICB 

```shell
duckdb -c "
  SELECT ods_code, name, postcode, icb_name
    FROM https://ods.fyi/orgs.parquet
   WHERE category = 'GP Practice';"
```

Work offline using `ods pull` to fetch the latest dataset to your computer.

```shell
ods pull
ods find --category 'GP Practice'
```

## ODS Data directory

The `ods` CLI manages a local workspace directory called `ods_data` to keep a local copy of each release you use, so you can always access it and refer back to it later.

The NHS TRUD ODS service releases updates every month. The parquet files are reproducibly derived from the official NHS TRUD ODS XML data, and are published automatically from CI each month.

`ods pull` fetches the parquet files and _provenance.json metadata and verifies the data integrity. 

Each release get's it's own date stamped folder, and symlink is set up to point `ods_data/current` to the last release you pulled.

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

## Contributing

Building from source requires Rust

```bash
cargo build
cargo test
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for more details.

Considerate PRs welcome! The priorities for this project are:

1. **Correctness & Provenance**
    - We must not introduce data errors, the trud xml is complicated enough
    - We must track sha256 hashes from NHS TRUD API to derived sources so researchers can trust and cite the derived data.
2. **User experience**
    - The NHS org data xml is hard to work with. We're publishing more a more user-friendly alternative - remotely queryable parquet files, and readable [Open Knowldege Format] compliant markdown.
     - Speed affects user experience. The tool must be fast.
     - Availability affects accessbility. The data must be available offline and not require a log in.


[Open Knowldege Format]: https://cloud.google.com/blog/products/data-analytics/how-the-open-knowledge-format-can-improve-data-sharing/

