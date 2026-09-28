# `ods.fyi`

All the organisations and sites in the NHS Organisation Data Service, as queryable & verifiable Parquet files.

The NHS Organisation Data Service (ODS) publishes reference data for every health and social care site. This project provides deterministic projections of that data to static Parquet files you can query offline with SQL, reproduce them yourself to verify, or rehost the data on any OCI container registry.

- `ods` is a Rust CLI that makes the ODS data easy to fetch, query, cite, and reproduce.
- [`ods.fyi`](https://ods.fyi) hosts the data as verifiable OCI Images. Republished to ghcr.io for resilience.

You can use the data in multiple ways:

| I want to...                  | Feature                | Commands |
|-------------------------------|------------------------|----------|
| _**Query** the data with SQL_ | Hosted Parquet files   | `SELECT * FROM 'https://ods.fyi/orgs.parquet' LIMIT 5` |
| _**Find** NHS orgs_           | Local-first search     | `ods pull` & `ods find`|
| _**Verify** the provenance_   | Reproducible builds    | `ods trud pull` & `ods make` |
| _**Cite** the data_           | Academic citation info | `ods cite` |
| _See what changed_            | Diff trud ods releases | `ods trud diff` |
| _Know how it works_           | See the SQL queries    | `ods find --sql` |

- **Open formats**: https://ods.fyi republishes it as Parquet files you can query with standard SQL. The tables can be queried over https via `duckdb`, or Python without installing `ods` at all.
- **Easy to query out of the box**: `ods find` gives you fast, local-first search across all NHS orgs and sites. Parent, PCN, ICB, Trust, and Successor relationships are all pre-calculated.
- **Reproducible projections**: Built directly from official NHS TRUD XML data with full provenance tracking and SHA-256 hash checks. You can build byte-for-byte identical projections yourself anytime with `ods make`.
- **Content-addressed**: https://ods.fyi publishes the datasets as OCI Images. `ods pull` automatically checks the file integrity for you, and will fetch identical bytes from ods.fyi or ghcr.io.

A sister project to `sct`, the SNOMED CT CLI. `ods` aims to provide an open, resilient, academically citable and easy to use source for NHS Org structure data.

> **NOTE** this is an independent project not commissioned by the NHS. The goal is to provide reliable and useful projections of NHS data, to aid research and to demonstrate the value of open formats and good UX.

## Query the data

- Install with `cargo install --locked --git https://github.com/olizilla/ods.git`.
- Use `duckdb` to query the hosted parquet files directly. 
- Use `ods pull` to fetch the data to your computer.
- Use `ods find` to explore the data without SQL.

### Try it out

`https://ods.fyi/orgs.parquet` is a convenience URL that follows the newest release.
Query the hosted Parquet files directly with `duckdb` without installing `ods`:

```shell
duckdb -c "
  SELECT ods_code, name, postcode, role_names
    FROM 'https://ods.fyi/orgs.parquet'
   WHERE status = 'active'
     AND (legal_end IS NULL OR legal_end > trud_release_date)
   AND list_contains(role_names, 'GP Practice')
   AND postcode LIKE 'SW9%';"
```

### Local-first

Running more than a few queries? Fetch the parquet files for the release once with `ods pull`

```bash
$ ods pull
  2026-08-28    ████████████████████  28MB   6 files  from ods.fyi  in 4.1s
  dataset       ods-data/2026-08-28_0.1.0
  verified      sha256 from releases.json
  linked        current → releases/2026-08-28

$ ods find --gp --in SW9
* Source: releases/2026-08-28/orgs.parquet
* --gp: RO76, RO227, RO315 — GP Practice, Scottish GP Practice, Northern Ireland GP Practice
┌──────────┬──────────────────────────────┬──────────┬────────────────┬───────┬──────────┐
│ ODS Code ┆ Name                         ┆ Postcode ┆ Roles          ┆ Class ┆ Matched  │
╞══════════╪══════════════════════════════╪══════════╪════════════════╪═══════╪══════════╡
│ G85028   ┆ STOCKWELL GROUP PRACTICE     ┆ SW9 9TJ  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ G85054   ┆ LAMBETH WALK GROUP PRACTICE  ┆ SW9 6AF  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ G85073   ┆ VASSALL MEDICAL CENTRE       ┆ SW9 6NA  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ G85100   ┆ BECKETT HOUSE PRACTICE       ┆ SW9 9DL  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ G85135   ┆ MINET GREEN HEALTH PRACTICE  ┆ SW9 6AF  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ G85695   ┆ AKERMAN MEDICAL PRACTICE     ┆ SW9 6AF  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ Y00020   ┆ THE GRANTHAM PRACTICE        ┆ SW9 9BH  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ Y03063   ┆ HETHERINGTON AT THE PAVILION ┆ SW9 8DJ  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ Y05161   ┆ FIVEWAYS PCN EA HUB          ┆ SW9 6AF  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ Y05163   ┆ LARC CLINIC (LA)             ┆ SW9 8DJ  ┆ GP Practice +1 ┆ org   ┆ postcode │
├──────────┴──────────────────────────────┴──────────┴────────────────┴───────┴──────────┤
│ 10 open                                                    Use --all to include closed │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

Local queries on static parquet files are _fast_! Each dataset is published as an [OCI Image](https://opencontainers.org/faq/), and `ods pull` verifies the file integrity for you.

## ODS data directory

The `ods` CLI manages a local workspace directory called `ods_data` to keep a local copy of each release you use, so you can always access it and refer back to it later.

The NHS TRUD ODS service releases updates every month. The parquet files are reproducibly derived from the official NHS TRUD ODS XML data, and each month's release is built and attested in CI, by [`build-dataset.yml`](.github/workflows/build-dataset.yml).

`ods pull` fetches the parquet files by their release's manifest, and verifies every byte against it.

Each release gets its own date stamped folder, and a symlink is set up to point `ods_data/current` to the last release you pulled.

```text
ods_data/
├── README.md                           # Workspace schema documentation
├── current -> releases/2026-07-31/     # Active release pin (symlink)
└── releases/
    └── 2026-07-31/                    # Release directory
        ├── datapackage.json           # Metadata as a standard [Data Package]
        ├── orgs.parquet               # Every org & site, active first (Primary analytical table)
        ├── relationships.parquet      # Target relationship links (ICB, Trust, PCN)
        ├── roles.parquet              # Primary & secondary roles (Holdings table)
        └── successions.parquet        # Entity successor chains
```

Each Parquet file carries its own provenance, so a single copied file still says which release it
is and whose data it holds. `datapackage.json` describes the release in the shape at
<https://datapackage.org/standard/data-package/>. See [docs/provenance.md](./docs/provenance.md).

See [docs/ods_data.md](./docs/ods_data.md) for full workspace specification details.

## Commands

```
Usage: ods <COMMAND>

Commands:
  find   Search NHS organisations and sites
  info   Show full details for a single organisation by ODS code
  role   Search and list role codes and names with holder counts
  pull   Download pre-built dataset releases
  cite   Show provenance metadata and academic citation
  trud   Build from official NHS source data (requires TRUD API key)
  make   Compile TRUD XML into Parquet tables
  use    Pin an active dataset release
  help   Print this message or the help of the given subcommand(s)
```


### find

Searches for organisations or sites in the Parquet tables by name, ODS code, or postcode.

```bash
ods find "Royal Free"
ods find --role 'GP Practice' --format csv
ods find --format tsv | fzf
```

### info

Shows full details for a single organisation by exact ODS code, including primary and secondary roles, contact details, hierarchy relationships, and resolved succession paths.

```bash
ods info A82608
ods info 0AF --format json
```

### pull

Pulls pre-compiled ODS Parquet release datasets into `./ods_data/`, lists available release versions, or switches active release pins.

```bash
ods pull                  # Pull latest release dataset
ods pull --list           # List all remote and local release versions (-l)
ods pull 2026-06-22       # Switch workspace pin or pull specific release
```

### cite

Shows how to cite the data, the `ods` tool and the NHS England source, with release provenance and SHA-256 hashes.

```bash
ods cite
```

### trud

Subcommands for working with the NHS TRUD REST API (`pull`, `diff`, `audit`, `verify`).

```bash
TRUD_API_KEY="<key>" ods trud pull
ods trud diff 2026-06-22 2026-07-31
ods trud audit
ods trud verify
```

### make

Generates target projections (Parquet tables) from raw TRUD XML data.

```bash
ods make                  # Generate target projections (Parquet tables)
ods make parquet          # Generate Parquet tables from TRUD XML
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
    - We must not introduce data errors, the trud xml is complicated enough.
    - We must track sha256 hashes from NHS TRUD API to derived sources so researchers can trust and cite the derived data.
    - Our output is deterministic. Anyone with an NHS TRUD account can reproduce the projections and verify the hashes match.
2. **User experience**
    - The NHS org data xml is fiddly to work with. We're publishing derived Parquet files as a more user-friendly alternative to allow SQL, querying over http, and queries across releases.
    - Speed affects user experience. The tool must be fast.
    - Availability is vital. The data must be available to use offline and refetchable from multiple sources, and not require a log in.

## Useful links

- ODS Data Model: https://www.odsdatasearchandexport.nhs.uk/referenceDataCatalogue/ODS-Data-Model_571324843.html
- NHS TRUD ODS info: https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5
- NHS TRUD ODS releases: https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases

## Licence

The ODS data is NHS England's, published under the [Open Government Licence (OGL)](https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/).

`ods` is [MIT](./LICENSE). To cite `ods`, run `ods cite` or see: [CITATION.cff](./CITATION.cff).

See: [CONTRIBUTING.md](./CONTRIBUTING.md#licensing) for more detail.


[Data Package]: https://datapackage.org/overview/introduction/
