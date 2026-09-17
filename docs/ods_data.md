# `ods_data` Workspace Directory

The `ods_data` directory is the local workspace data root managed by the `ods` CLI. It organizes raw TRUD ODS releases, cryptographic provenance metadata, and derived projection artifacts (Parquet tables and NDJSON streams).

## Workspace Directory Tree

When initialized by `ods trud pull` or workspace-aware subcommands, `ods_data` maintains an immutable release archive with an active release pin symlink:

```text
ods_data/
├── README.md                           # Auto-generated workspace query guide & schema documentation
├── .gitignore                           # Ignores raw downloads (releases/*/trud/, *.zip, *.xml)
├── current -> releases/2026-07-31/      # Active workspace release pin (symlink)
└── releases/
    └── 2026-07-31/                     # Immutable release directory (named by release date)
        ├── _provenance.json            # Source TRUD metadata & SHA-256 derived hashes
        ├── orgs.parquet                # Active organisations & sites table (Primary surface)
        ├── orgs_all.parquet            # Complete historical orgs table (active + inactive)
        ├── relationships.parquet       # Target relationship links (ICB, Trust, Region, PCN)
        ├── roles.parquet              # Primary & secondary role mappings
        ├── successions.parquet        # Entity successor chains & reorganisations
        ├── datapackage.json           # Frictionless Data Package descriptor
        └── trud/                      # Official TRUD zip archives (gitignored)
            └── hscorgrefdataxml_data_7.0.0_20260731000001.zip
```

## `_provenance.json` Specification & Schema

Every release folder contains a `_provenance.json` file recording the integrity, distribution parameters, inner XML payload attributes, and derived artifact hashes of the dataset.

### Provenance Key Namespaces

`provenance.json` records metadata across three distinct namespaces:

#### Distribution System Metadata (`trud_release_*`)
- **Origin**: Fetched directly from the **NHS TRUD REST API** (`/items/341/releases`).
- **Scope**: Represents the **distribution container / envelope** published on TRUD.
- **Fields**:
  - `trud_release_name`: TRUD's release label. Not unique. Not semver. `"Release 7.0.0"`).
  - `trud_release_date`: Official release date in `YYYY-MM-DD` format (e.g. `"2026-07-31"`).
  - `trud_release_file`: TRUD archive ZIP filename (e.g. `"hscorgrefdataxml_data_7.0.0_20260731000001.zip"`).
  - `trud_release_filesize_bytes`: Archive file size in bytes (`37983173`).
  - `trud_release_sha256`: Published SHA-256 checksum from NHS TRUD.
  - `trud_release_sha256_verified`: Verification status (`"trud_api"`, `"published_release"`, or `"unverified"`).

#### Inner Data Content Metadata (`publication_*`)
- **Origin**: Extracted from the `<un:ManifestHeader>` tag inside the inner XML file (`HSCOrgRefData.xml`).
- **Scope**: Represents the **domain data publication attributes** embedded within the XML payload.
- **Fields**:
  - `publication_date`: Date the XML snapshot was generated (e.g. `"2026-07-28"`).
  - `publication_seq_num`: Sequential publication run sequence number (e.g. `"4700"`).
  - `publication_type`: Release type (`"Full"` or `"Incremental"`).
  - `publication_source`: Issuing authority (`"HSCIC"` / `"NHS England"`).
  - `publication_record_count`: Number of organisation records declared in XML manifest.

#### Tool and Dataset Metadata (`tool_*`, `dataset_*`)
- `tool_git_sha`: Git commit SHA of the `ods` binary that generated the projections.
- `tool_git_dirty`: Optional boolean present when built from an uncommitted working tree.
- `tool_version`: Cargo package version of `ods` (`"0.1.0"`).
- `dataset_version`: SemVer identifying the dataset schema cut and release attempt (`"0.1.0"`).

### Example `_provenance.json`

```json
{
  "_type": "ods_provenance",
  "trud_release_name": "Release 7.0.0",
  "trud_release_date": "2026-07-31",
  "trud_release_file": "hscorgrefdataxml_data_7.0.0_20260731000001.zip",
  "trud_release_filesize_bytes": 37983173,
  "trud_release_sha256": "8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933",
  "trud_release_sha256_verified": "trud_api",
  "publication_date": "2026-07-28",
  "publication_seq_num": "4700",
  "publication_type": "Full",
  "publication_source": "HSCIC",
  "publication_schema_version": "2-0-0",
  "publication_record_count": 305541,
  "tool_git_sha": "f0630a986ca87489933bfb528214434329e2c02f",
  "tool_version": "0.1.0",
  "dataset_version": "0.1.0"
}
```

## Cryptographic SHA-256 Verification Flow

```mermaid
flowchart TD
    A[ods trud pull --api-key KEY] --> B[Query TRUD REST API /items/341/releases]
    B --> C[Fetch Official archiveFileSha256 & archiveFileUrl]
    C --> D[Download Archive .zip to ods_data/releases/DATE/trud/]
    D --> E[Compute Local SHA-256 Hash]
    E --> F{Local SHA-256 == Official TRUD SHA-256?}
    F -- Match --> G[✓ Set trud_release_sha256_verified = true in provenance.json]
    F -- Mismatch --> H[Retry once -> If fail, rename bad file to .zip.bad-sha]
```

## Command Integration & Workspace Lifecycle

| Subcommand | Interaction with `ods_data` |
| :--- | :--- |
| **`ods trud list`** | Queries TRUD API and lists available release archives, indicating which are held in the workspace. |
| **`ods trud pull`** | Queries TRUD API, downloads ZIP into `releases/<date>/trud/`, verifies SHA-256, writes `provenance.json`, and updates `current` symlink. |
| **`ods cite`** | Displays active release pin, provenance metadata, local release versions, disk space, and SHA-256 verification status. |
| **`ods pull`** | Pulls pre-built dataset release or switches active release pin to target release date. |
| **`ods trud audit`** | Validates workspace Parquet tables against ground-truth TRUD XML/ZIP archives and verifies release provenance alignment. |
| **`ods find`** | Queries active workspace Parquet tables in `ods_data/current/parquet/` automatically. |
