# `ods_data` workspace directory

The `ods_data` directory is the local workspace data root managed by the `ods` CLI. It organises raw TRUD ODS releases, provenance metadata, and derived projection artefacts (Parquet tables).

## Workspace directory tree

`ods pull` creates an ods_data dir in the current working directy to keep monthly the monthly releases
in a well known order, so you don't have to manage it, and so we can share interesting queries that
glob over multiple months datasets. See: [queries.md](./queries.md)

```text
ods_data/
├── current -> releases/2026-07-31/     # Symlink for the release for ods commands should use.
└── releases/
    └── 2026-07-31/                     
        ├── datapackage.json            # The release's Data Package view, written by `ods make` or `ods pull`
        ├── orgs.parquet                # All the orgs
        ├── relationships.parquet       # Target relationship links (ICB, Trust, Region, PCN)
        ├── roles.parquet               # Primary & secondary role mappings
        ├── successions.parquet         # Entity successor chains & reorganisations
        #
        └── trud/                       # Only present after `ods trud pull`
            ├── hscorgrefdataxml_data_7.0.0_20260731000001.zip            # The source!
            ├── trud_hscorgrefdataxml_data_7.0.0_20260731000001.xml       # SHA1 checksum of zip
            ├── trud_hscorgrefdataxml_data_7.0.0_20260731000001.xml.asc   # Signed checksum
            ├── trud-public-key-2013-04-01.pgp                            # NHS's public key
            └── datapackage.json                                          # Metadata
```

A release fetched with `ods pull` holds the four Parquet files and `datapackage.json`.

## Provenance lives in the Parquet files

Every Parquet file carries the release's provenance in one key-value metadata key,
`datapackage`: the dataset's name and version, its licence and attribution, and the TRUD release
it was built from, with its SHA-256 and size. It's the one record `ods` reads. The OCI manifest is
rebuilt from the files whenever `ods` verifies a release, and `datapackage.json` is a view of them
that nothing reads back.

Which `ods` built a published dataset is recorded in its release index row (`tool_version`, 
`tool_git_sha`,see [release-index.md](./release-index.md)) and in the CI attestation, not here. 

See [provenance.md](./provenance.md) for more details

## Command integration and workspace lifecycle

| Command | Interaction with `ods_data` |
| :--- | :--- |
| **`pull`** | Pulls pre-built dataset release or switches active release pin to target release date. |
| **`find`** | Queries active workspace Parquet tables in `ods_data/current/orgs.parquet` automatically. |
| **`cite`** | Displays active release pin, provenance metadata, local release versions, disk space, and SHA-256 verification status. |
| **`trud pull`** | Queries TRUD API, downloads ZIP into `releases/<date>/trud/`, verifies SHA-256, writes the pull record `trud/datapackage.json` for `ods make` to build from, and updates `current` symlink. |
| **`trud audit`** | Validates workspace Parquet tables against ground-truth TRUD XML/ZIP archives and verifies release provenance alignment. |

