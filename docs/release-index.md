# ods release index

The release index records TRUD's source releases and the datasets we've built from each.

It's baked into every `ods` binary at compile time (`data/releases.json`), served at `https://ods.fyi/releases.json`, and cached in each workspace as `_releases.json`.

## Example

```json
{
  "$schema": "https://ods.fyi/schema/releases.v1.json",
  "trud_signing_key_fingerprint": "71ED5964BAE53E83556320A42BE59DADEE84BEB0",
  "mirrors": [
    { "url": "https://ods.fyi/v2/ods-data" },
    { "url": "https://ghcr.io/v2/olizilla/ods-data" }
  ],
  "releases": [
    {
      "trud_release_date": "2026-08-28",
      "trud_release_sha256": "ABDD194B1569D5FF3CDD81D618847F05642BD43C5B15D6CD43D8289B7466D801",
      "trud_release_filesize_bytes": 38064419,
      "datasets": [
        { "dataset_version": "0.1.0", "manifest_digest": "sha256:ba543ac4856f62d186c8fbeae1579d46e9196b2fd5268c5b96788db3964821a8" }
      ]
    }
  ]
}
```

Every field and its constraints are defined in [releases.v1.json](https://ods.fyi/schema/releases.v1.json) (or in the repo at `worker/schema/releases.v1.json`).

## The rules

- **Append-only releases.** A release date is never removed from the index. Once recorded, its hash and size can never change.
- **Datasets are blessed; releases are recorded.** Trust rules — manifest digests, citations, withdrawal — apply to datasets. A release row records what TRUD published. A release may have no datasets yet, and every reader treats an empty list as normal.
- **Mirrors are places to look.** They are an ordered list for the whole index. Content digests name the bytes wherever they're stored, so a miss on the first mirror falls through to the next.
- **The pinned fingerprint anchors trust.** `trud_signing_key_fingerprint` pins NHS England's 40-hex PGP key (`71ED5964BAE53E83556320A42BE59DADEE84BEB0`), so signatures can be checked against it even after TRUD is decommissioned.
- **What merge refuses.** When `ods` fetches an updated index from ods.fyi or GitHub, `merge` will refuse it with a security error if:
  - The signing-key fingerprint differs from the baked-in key.
  - A known TRUD release changes its `trud_release_sha256` or `trud_release_filesize_bytes`.
  - An existing dataset changes its `manifest_digest`.
  Fetched indexes may append new releases, add new datasets, or update metadata such as `withdrawn` notices and DOIs.

## Naming

Formats we own that travel alone carry `$schema` as their identifier. Facts carry namespace prefixes (`trud_`, `publication_`, `dataset_`, `tool_`).

A release uses `trud_release_date`, `trud_release_sha256`, and `trud_release_filesize_bytes` — named identically to `_provenance.json`, so one fact has one name everywhere. Datasets carry `dataset_version`, `manifest_digest`, and optional `dataset_doi` and `withdrawn`.

## Checking the index

The copy in the GitHub repository (`data/releases.json`) is canonical. `https://ods.fyi/releases.json` serves a copy of it and is tried first because it's faster.

Anyone can verify that the served copy matches git:

```bash
diff <(curl -s https://ods.fyi/releases.json) <(curl -s https://raw.githubusercontent.com/olizilla/ods/main/data/releases.json)
```

The GitHub raw URL works once the repository is public.
