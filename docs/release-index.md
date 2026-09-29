# ods release index

The release index records a dataset family's source releases and the datasets we've built from each. For `ods-data`, the source is TRUD's NHS ODS XML releases.

It's baked into every `ods` binary at compile time (`data/releases.json`), served at `https://ods.fyi/releases.json`, and cached in each workspace as `_releases.json`.

## Example

```json
{
  "$schema": "https://ods.fyi/schema/releases.v1.json",
  "name": "ods-data",
  "source": {
    "title": "NHS Organisation Data Service XML Data",
    "path": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
    "signing_key_fingerprints": [
      "71ED5964BAE53E83556320A42BE59DADEE84BEB0"
    ]
  },
  "mirrors": [
    { "url": "https://ods.fyi/v2/ods-data" },
    { "url": "https://ghcr.io/v2/olizilla/ods-data" }
  ],
  "releases": [
    {
      "source": {
        "version": "2026-09-25",
        "hash": "sha256:ca0fee7512f593ada1fa9b95bf1372b41911167da463a98fecf33adfd86697e5",
        "bytes": 38138574
      },
      "datasets": [
        {
          "version": "2026-09-25_0.1.0",
          "manifest_digest": "sha256:ba543ac4856f62d186c8fbeae1579d46e9196b2fd5268c5b96788db3964821a8",
          "bytes": 34326790,
          "tool_version": "0.2.0",
          "tool_git_sha": "0123456789abcdef0123456789abcdef01234567"
        }
      ]
    },
    {
      "source": {
        "version": "2025-05-30",
        "hash": "sha256:b54caf77e699781b5294b209747f8658a3cdf0a4ee3ca917df2373344b7c1bcc",
        "bytes": 36377546,
        "issues": ["fef03-code-reused"]
      },
      "datasets": []
    }
  ]
}
```

Every field and its constraints are defined in [releases.v1.json](https://ods.fyi/schema/releases.v1.json) (or in the repo at `worker/schema/releases.v1.json`).

## The rules

- **Append-only releases.** A release is never removed from the index. Once recorded, its source `version`, `hash` and `bytes` can never change.
- **Datasets are blessed; releases are recorded.** Trust rules — manifest digests, citations, withdrawal — apply to datasets. A release row records what the source published. A release may have no datasets yet, and every reader treats an empty list as normal. The index records a release when a dataset built from it is recorded, or when it has a known source issue: `ods make release` adds the row for the release it's recording, from the object its Parquet files carry (see [provenance.md](./provenance.md)), and reads no listing of other source releases, so every other release gets its row with its first dataset or its first issue.
- **Each dataset row names the `ods` that built it.** `tool_version` is the tool's tag without its `v`, and `tool_git_sha` the 40-character commit that tag points at. `ods make release` writes both once, from its own build, and refuses to record when that `ods` was built from a dirty tree, without a commit, or from a commit that isn't the one its tag names. They never change once written: `merge` refuses a contradiction like a contradicting digest, and an identical re-publish by another `ods` leaves the row as the first one wrote it. The pair is the convenient, git-reviewed pointer to which build made a dataset. The signed proof is the CI attestation for the same manifest digest. A dataset's bytes don't depend on the tool: any `ods` at the same dataset version rebuilds them, so these fields are for tracing a dataset, not for identifying it.
- **Known source issues are recorded, and never removed.** A release's `source.issues` lists the IDs of known errors in what the source published, each described, with how `ods` resolves it, at [docs/source-issues/](./source-issues/README.md)`<id>.md`. The index holds IDs only, never URLs, so no location is frozen into it. Like `withdrawn`, the field may appear on a row later and is never removed. Rows gain it by hand, in `data/releases.json`. `ods cite` and `ods pull` name each issue with its page.
- **Mirrors are places to look.** They are an ordered list for the whole index. Content digests name the bytes wherever they're stored, so a miss on the first mirror falls through to the next.
- **The selected index decides.** Commands select a single index by precedence: `--index`, then fetched from ods.fyi or GitHub, then the workspace cache (`_releases.json`), then the index built into `ods`. The signing-key fingerprints are the selected index's, with no separate compiled copy in `ods` to compare them against. The set names every key the source publisher has signed with (ordered oldest first), and a signature from any key in the set is accepted; a key rotation is recorded by adding the new fingerprint to the set. A source that signs nothing omits `signing_key_fingerprints`.
- **Append-only enforced at publish time.** `ods make release` checks that a candidate release index may append rows and metadata but may never contradict an existing source hash or size, or a manifest digest. Clients trust the selected index; the manifest a local release directory's files rebuild, compared with its row, is what catches a changed dataset locally.

## Naming

Formats we own that travel alone carry `$schema` as their identifier. A generated `datapackage.json` view follows Data Package v2.

One fact has one name wherever it appears: the index, the Parquet files' embedded `datapackage` object, the `datapackage.json` view and the OCI manifest all use the Data Package vocabulary.

| Fact | Index | Parquet metadata and view | Manifest and tag |
| :--- | :--- | :--- | :--- |
| the dataset family | `name` | `name` | the repository, `org.opencontainers.image.title` |
| the source family | `source.title`, `source.path` | `sources[0].title`, `sources[0].path` | |
| the source release | `releases[].source.version` | `sources[0].version` | the date in the tag |
| its archive's hash | `releases[].source.hash` | `sources[0].hash` | |
| its archive's size | `releases[].source.bytes` | `sources[0].bytes` | |
| the dataset | `datasets[].version` | `version` | `org.opencontainers.image.version`, the tag |

Hashes are `sha256:` and lower-case hex everywhere. A dataset's `version` is `<source version>_<dataset version>`, e.g. `2026-09-25_0.1.0`: the part after the `_` is the dataset version, the SemVer that says which schema the files have ([parquet.md](./parquet.md)). A dataset row's `bytes` is the sum of its manifest's layer sizes, what `ods pull` downloads. `manifest_digest`, `tool_version` and `tool_git_sha` (the `ods` that built it), and the optional `doi` and `withdrawn`, are the index's own. Nothing in the shape is particular to TRUD, so the same index serves every dataset family.

## How a dataset gets into the index

**CI is the hash oracle.** `.github/workflows/build-dataset.yml` builds each date twice, on
`macos-latest` and `ubuntu-latest`, from the same NHS archive cached in
`ghcr.io/olizilla/nhs-ods-xml`. Only a date whose two manifest digests agree is pushed, re-pulled
from ghcr.io and checked, and attested. The run ends with a candidate `releases.json`. The
maintainer tries it with `ods pull --index`, mirrors it to ods.fyi, and commits it as
`data/releases.json`. That commit is what blesses a dataset. The steps are in
[CONTRIBUTING.md](../CONTRIBUTING.md#dataset-releases).

**Where facts live**

| Where | What it holds |
| :--- | :--- |
| Blobs, by digest | the bytes |
| Each Parquet file's `datapackage` metadata | the dataset's name and version, the licence and attribution, and the TRUD release it came from, with its hash and size |
| The manifest | nothing of its own: a function of the Parquet files ([oci.md](./oci.md)) |
| The index row | that the dataset is blessed, whether it's withdrawn, its DOI, the `ods` that built it, and any known issues with its source release |
| The index's `mirrors` | where to fetch it |
| Registry tags | a name for the dataset on that registry, for OCI tools |

**A tag doesn't bless anything.** Registry tags name datasets for `oras`, `docker` and other OCI
tools, and `ods` never reads one: it finds datasets only through the index. The tags and their
rules are in [oci.md](./oci.md#tags).

**Credentials stay split.** GitHub holds the TRUD API key and the workflow's `GITHUB_TOKEN`, which
can push to ghcr.io. Cloudflare holds nothing from GitHub. The token that writes to ods.fyi's R2
bucket exists only on the maintainer's laptop. So CI can't publish to ods.fyi, and ods.fyi can't
publish to ghcr.io.

## Checking the index

The copy in the GitHub repository (`data/releases.json`) is canonical. `https://ods.fyi/releases.json` serves a copy of it and is tried first because it's faster.

Anyone can verify that the served copy matches git:

```bash
diff <(curl -s https://ods.fyi/releases.json) <(curl -s https://raw.githubusercontent.com/olizilla/ods/main/data/releases.json)
```

The GitHub raw URL works once the repository is public.

## Verifying a dataset's attestation

Every dataset was pushed and attested by `.github/workflows/build-dataset.yml` (R9), so a reader
can check that directly, without trusting this index: [oci.md](./oci.md#attestations) has the
command and what it proves.
