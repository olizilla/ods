# Changelog

All notable changes to the `ods` tool will be documented in this file.

## [0.2.1] - 2026-09-29

Builds dataset 0.2.0.

### Fixed
- `ods pull` works with ghcr.io: it asks for OCI manifests and follows blob redirects.
- CI: the macOS witness logs in with oras and runs bash 5

### Added
- CI rehearsals run from any ref; `ods make release --rehearsal` skips only the tag check.

## [0.2.0] - 2026-09-29

Builds dataset 0.2.0.

### Added
- `ods trud list`, `ods trud pull` and `ods cite`.
- Every Parquet file carries its own provenance: licence, attribution and the TRUD release it came from.
- One `orgs` table from both of NHS's XML files, with `publication_date` on every row.
- Datasets are built in public CI, attested, and mirrored to ghcr.io.
- Known source issues, recorded in the release index and shown by `ods cite` and `ods pull`.

### Changed
- `find` and `role` show open organisations by default.
- The release index and `datapackage.json` share one vocabulary.

### Removed
- Top-level `ods diff` and `ods audit`; use `ods trud diff` and `ods trud audit`.

## [0.1.0] - 2026-08-27

### Added
- Initial release with TRUD fetching, Parquet compilation, OCI dataset packaging, and verification.
