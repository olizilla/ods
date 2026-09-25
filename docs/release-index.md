# ods release index

The release index records TRUD's source releases and the datasets we've built from each.

It's baked into every `ods` binary at compile time (`data/releases.json`), served at `https://ods.fyi/releases.json`, and cached in each workspace as `_releases.json`.

## Example

```json
{
  "$schema": "https://ods.fyi/schema/releases.v1.json",
  "trud_signing_key_fingerprints": [
    "71ED5964BAE53E83556320A42BE59DADEE84BEB0"
  ],
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
        {
          "dataset_version": "0.1.0",
          "manifest_digest": "sha256:ba543ac4856f62d186c8fbeae1579d46e9196b2fd5268c5b96788db3964821a8",
          "dataset_filesize_bytes": 34326790,
          "tool_version": "0.2.0",
          "tool_git_sha": "0123456789abcdef0123456789abcdef01234567"
        }
      ]
    }
  ]
}
```

Every field and its constraints are defined in [releases.v1.json](https://ods.fyi/schema/releases.v1.json) (or in the repo at `worker/schema/releases.v1.json`).

## The rules

- **Append-only releases.** A release date is never removed from the index. Once recorded, its hash and size can never change.
- **Datasets are blessed; releases are recorded.** Trust rules — manifest digests, citations, withdrawal — apply to datasets. A release row records what TRUD published. A release may have no datasets yet, and every reader treats an empty list as normal.
- **Each dataset row names the `ods` that built it.** `tool_version` is the tool's tag without its `v`, and `tool_git_sha` the 40-character commit that tag points at. `ods make release` writes both once, from its own build, and refuses to record when that `ods` was built from a dirty tree, without a commit, or from a commit that isn't the one its tag names. They never change once written: `merge` refuses a contradiction like a contradicting digest, and an identical re-publish by another `ods` leaves the row as the first one wrote it. The pair is the convenient, git-reviewed pointer to which build made a dataset. The signed proof is the CI attestation for the same manifest digest. A dataset's bytes don't depend on the tool: any `ods` at the same `dataset_version` rebuilds them, so these fields are for tracing a dataset, not for identifying it.
- **Mirrors are places to look.** They are an ordered list for the whole index. Content digests name the bytes wherever they're stored, so a miss on the first mirror falls through to the next.
- **The selected index decides.** Commands select a single index by precedence: `--index`, then fetched from ods.fyi or GitHub, then the workspace cache (`_releases.json`), then the index built into `ods`. The signing-key fingerprints are the selected index's, with no separate compiled copy in `ods` to compare them against. The set names every key NHS England has signed with (ordered oldest first), and a signature from any key in the set is accepted; a key rotation is recorded by adding the new fingerprint to the set.
- **Append-only enforced at publish time.** Contradiction checking moves to `ods make release`, where candidate release indexes may append rows and metadata but may never contradict existing TRUD hashes, sizes, or manifest digests. Clients trust the selected index; a local release directory's own manifest digest remains what catches a changed dataset locally.

## Naming

Formats we own that travel alone carry `$schema` as their identifier. Facts carry namespace prefixes (`trud_`, `dataset_`, `tool_`). `datapackage.json` follows Data Package v2.

A release uses `trud_release_date`, `trud_release_sha256`, and `trud_release_filesize_bytes` — named identically to `_provenance.json`, so one fact has one name everywhere. Datasets carry `dataset_version`, `manifest_digest`, `dataset_filesize_bytes` (the sum of the manifest's layer sizes — what `ods pull` downloads), `tool_version` and `tool_git_sha` (the `ods` that built it), and optional `dataset_doi` and `withdrawn`.

## Checking the index

The copy in the GitHub repository (`data/releases.json`) is canonical. `https://ods.fyi/releases.json` serves a copy of it and is tried first because it's faster.

Anyone can verify that the served copy matches git:

```bash
diff <(curl -s https://ods.fyi/releases.json) <(curl -s https://raw.githubusercontent.com/olizilla/ods/main/data/releases.json)
```

The GitHub raw URL works once the repository is public.
