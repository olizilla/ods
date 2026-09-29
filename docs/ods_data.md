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

## How `ods` finds the workspace

A command that isn't told where the workspace is looks where you are standing:

- The current directory is a workspace root, or holds `ods_data/`: that is the workspace.
- The current directory is inside a workspace at a path it defines: `releases/`, `releases/<date>/`,
  `releases/<date>/trud/`, `releases/<date>/trud/oci/` and below, or `current`. The workspace root
  above it is used, so you can run `ods` from a release directory.
- Anywhere else there is no workspace, even when a parent directory has an `ods_data/`. Commands that
  read say so and name `ods pull`; `ods pull` and `ods trud pull` create `./ods_data` in the
  directory you ran them from.

`ods` doesn't search parent directories for a workspace: a scratch directory made inside a project
gets its own `ods_data/` when you pull there, rather than filling the project's. Name a workspace from
anywhere with `--workspace <dir>` (where a command has it) or `-i <release dir>`.

## Provenance lives in the Parquet files

Every Parquet file carries the release's provenance in one key-value metadata key,
`datapackage`: the dataset's name and version, its licence and attribution, and the TRUD release
it was built from, with its SHA-256 and size. It's the one record `ods` reads. The [OCI
manifest](./oci.md) is rebuilt from the files whenever `ods` verifies a release, and
`datapackage.json` is a view of them that nothing reads back ([datapackage.md](./datapackage.md)).

Which `ods` built a published dataset is recorded in its release index row (`tool_version`, 
`tool_git_sha`,see [release-index.md](./release-index.md)) and in the CI attestation, not here. 

See [provenance.md](./provenance.md) for more details

## Command integration and workspace lifecycle

| Command | Interaction with `ods_data` |
| :--- | :--- |
| **`pull`** | Pulls pre-built dataset release or switches active release pin to target release date. |
| **`find`** | Queries active workspace Parquet tables in `ods_data/current/orgs.parquet` automatically. |
| **`cite`** | Displays active release pin, provenance metadata, local release versions, disk space, and SHA-256 verification status. |
| **`trud pull`** | Queries TRUD API, downloads ZIP into `releases/<date>/trud/`, verifies SHA-256, writes the TRUD archive package, `trud/datapackage.json`, for `ods make` to build from, and updates `current` symlink. |
| **`use`** | Pins a release in the workspace, or the newest built one with `latest`. A release pulled from TRUD and not built pins too, and `ods use` says to run `ods make`; `ods use latest` with only such releases lists them and how to build the newest. |
| **`make`** | Builds the active release into itself. `ods make -i <release dir>` (or its `trud/`) builds that release into that release, whichever is active. `-o` names another directory. |
| **`trud audit`** | Validates workspace Parquet tables against ground-truth TRUD XML/ZIP archives and verifies release provenance alignment. |

