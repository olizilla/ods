# Provenance

Every Parquet file `ods` writes where it came from in it's metadata. 
A loose `orgs.parquet` is still citable and DuckDB can read it without `ods`:

```console
$ duckdb -noheader -list -c "SELECT decode(value) FROM parquet_kv_metadata('orgs.parquet') WHERE decode(key) = 'datapackage'"
{"name":"ods-data","version":"2026-09-25_0.2.0","licenses":[{"name":"OGL-UK-3.0","path":"https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/","title":"Open Government Licence v3.0","attribution":"Contains information from NHS England, licensed under the current version of the Open Government Licence."}],"contributors":[{"title":"NHS England","roles":["rightsHolder"]},{"title":"NHS TRUD","roles":["distributor"]}],"sources":[{"title":"NHS Organisation Data Service XML Data","version":"2026-09-25","path":"https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases","hash":"sha256:ca0fee7512f593ada1fa9b95bf1372b41911167da463a98fecf33adfd86697e5","bytes":38138574}]}
```

Everything `ods` says about a release comes from there. The file hashes in the [OCI
image](./oci.md) mean it can't change without the release's digest changing too.

## The embedded object

One key-value metadata key, `datapackage`, holding compact JSON in [Data Package][datapackage]
vocabulary, with its keys in this order. All four files carry the same object:

```json
{
  "name": "ods-data",
  "version": "2026-09-25_0.2.0",
  "licenses": [
    {
      "name": "OGL-UK-3.0",
      "path": "https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/",
      "title": "Open Government Licence v3.0",
      "attribution": "Contains information from NHS England, licensed under the current version of the Open Government Licence."
    }
  ],
  "contributors": [
    { "title": "NHS England", "roles": ["rightsHolder"] },
    { "title": "NHS TRUD", "roles": ["distributor"] }
  ],
  "sources": [
    {
      "title": "NHS Organisation Data Service XML Data",
      "version": "2026-09-25",
      "path": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
      "hash": "sha256:ca0fee7512f593ada1fa9b95bf1372b41911167da463a98fecf33adfd86697e5",
      "bytes": 38138574
    }
  ]
}
```

| Field | Description | Source |
| :--- | :--- | :--- |
| `name` | The dataset family, `ods-data` | `ods` |
| `version` | `<source release>_<dataset version>` same as the OCI tag | `ods` and NHS TRUD |
| `licenses[0].name`, `.path`, `.title` | The licence SPDX id, URL, and title | NHS TRUD |
| `licenses[0].attribution` | The attribution per the license terms | NHS TRUD |
| `contributors` | Who holds the rights (`rightsHolder`) and who distributes (`distributor`), as DataCite roles | NHS TRUD |
| `sources[0].title` | The source release's title | NHS TRUD |
| `sources[0].version` | The source release: TRUD's release date | NHS TRUD |
| `sources[0].path` | The landing page for the item | NHS TRUD |
| `sources[0].hash`, `.bytes` | The SHA-256 and size | NHS TRUD |

It holds facts that are true about the bytes, plus the upstream's own URLs. 

Nothing about which `ods` built a release, or how the zip was checked, is in it so any `ods` that builds the same zip at the same dataset version writes the same bytes.

The `ods` that built a published dataset is on its [release index](./release-index.md) row
(`tool_version`, `tool_git_sha`) and in the CI attestation for its manifest digest.

Files that don't carry the same provenance are flagged by `ods` commands. Here `orgs.parquet`'s footer was edited to say `0.2.1`, as if it came from another build:

```console
$ ods use 2026-09-25
✖ The Parquet files in ods_data/releases/2026-09-25 don't carry the same provenance
  orgs.parquet           2026-09-25_0.2.1  (datapackage sha256:6b811daf9c43…)
  relationships.parquet  2026-09-25_0.2.0  (datapackage sha256:06a0630a20c6…)
  roles.parquet          2026-09-25_0.2.0  (datapackage sha256:06a0630a20c6…)
  successions.parquet    2026-09-25_0.2.0  (datapackage sha256:06a0630a20c6…)
  A release's files all come from one build. Pull it again with `ods pull --force`, or rebuild it with `ods make`.
```

## One vocabulary for every dataset

| Where | Names |
| :--- | :--- |
| The object in the Parquet files, and the two descriptors ([datapackage.md](./datapackage.md)) | [Data Package][datapackage] property names: `name`, `version`, `licenses`, `contributors`, `sources`, `resources`, plus `licenses[].attribution` and the Source's `hash`, `bytes` and `_cache` from ods's profile |
| The OCI manifest ([oci.md](./oci.md)) | the image spec's own keys only: `org.opencontainers.image.title`, `.version`, `.description`, `.licenses`, `.created`, `.source` |
| The release index ([release-index.md](./release-index.md)) | the same names for the same facts: `name`, `source` (`title`, `path`), and each release's `source` (`version`, `hash`, `bytes`) and datasets' `version` and `bytes`, plus the index's own `manifest_digest`, `tool_version`, `tool_git_sha`, `doi`, `withdrawn` and `source.issues` |

The source's facts (its title, TRUD's release date, the zip's SHA-256 and size) are read from a
Parquet file's footer, not from the manifest:

```console
$ duckdb -noheader -list -c "SELECT decode(value)::JSON -> '$.sources[0]' FROM parquet_kv_metadata('orgs.parquet') WHERE decode(key) = 'datapackage'"
{"title":"NHS Organisation Data Service XML Data","version":"2026-09-25","path":"https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases","hash":"sha256:ca0fee7512f593ada1fa9b95bf1372b41911167da463a98fecf33adfd86697e5","bytes":38138574}
```

`ods` verifies a release by rebuilding its OCI manifest from the files and comparing the digest
with its release index row ([oci.md](./oci.md#the-manifest-is-a-function-of-the-files)).
`ods make` and `ods pull` also write a readable view of the release, `datapackage.json`, and `ods
trud pull` records the TRUD release it downloaded as the TRUD archive package, `trud/datapackage.json`: both are described
in [datapackage.md](./datapackage.md).

Parquet files an older `ods` built carry no `datapackage` key, like a `--force` build's, and
`ods` can't tell the two apart. `find`, `info` and `role` read them with a warning, and `cite`
refuses them. `ods trud pull <date> --force && ods make` rebuilds them, and `ods pull --force
<date>` fetches the published ones.

## Unmatched archives

When an archive can't be matched by SHA-256 to a row in the release index, or to TRUD's API when
`TRUD_API_KEY` is set:

- **`ods make <zip>` refuses it, unless you pass `--force`.** Building an unvouched archive is a
  deliberate act:
  ```console
  ✖ hscorgrefdataxml_data_8.0.0_20260828000001.zip isn't a TRUD release ods knows (SHA-256 ...)
    Get it through ods trud pull, or run ods pull for a newer release index.
    To build it anyway, without provenance: ods make -i hscorgrefdataxml_data_8.0.0_20260828000001.zip -o <dir> --force
  ```
- **`--force` builds it only outside the workspace, with `-o <dir>`.** `--force` alone refuses:
  ```console
  ✖ --force builds outside the workspace only
    Add -o <dir>: a workspace holds only releases with provenance.
  ```
- **`ods make <zip> -o <dir> --force` builds it without provenance.** The files carry no
  `datapackage` key: for an archive nobody could match, a name, a licence or a source would be a
  plausible wrong answer. The zip can have any name, since nothing is read from it:
  `publication_date` comes from the XML as in every build. A zip without `fullfile.zip` and
  `archive.zip` inside is refused, naming what's missing. The build warns after its block:
  ```console
  ! hscorgrefdataxml_data_9.0.0_20260925000001.zip isn't a TRUD release ods knows (SHA-256 CA0FEE75…97E5)
    Built without provenance. You can explore it with find, info and role, but not cite or publish it.
  ```
- **`ods trud pull --local-archive <zip>` refuses it:** `Build it outside the workspace with -o
  <dir>, or run ods pull for a newer release index.` It has no `--force` of its own.
- **`find`, `info` and `role` read it, with one warning:**
  ```console
  $ ods find --code RJZ -i force
  ! force's Parquet files carry no provenance: it was built from an archive ods couldn't match to a TRUD release, or by an older ods
    You can explore it, but not cite or verify it.
  ```
- **`cite`, `make oci`, `make release` and `trud audit` refuse it:**
  ```console
  $ ods cite -i force
  ✖ force's Parquet files carry no provenance: it was built from an archive ods couldn't match to a TRUD release, or by an older ods
    To cite or publish it, get the archive through ods trud pull and build it with ods make, or pull the release with ods pull.
  ```

## NHS's signature

Alongside each archive, TRUD serves a checksum file naming the zip and its SHA-1, NHS England's
PGP signature of that file, and NHS's public key. `ods trud pull` downloads all three, and the
TRUD archive package names each with its hash. `scripts/verify-trud-bundle.sh` checks them: the key's
fingerprint is pinned in the release index (`source.signing_key_fingerprints`), the signature is
good, and the signed SHA-1 is the zip's. `build-dataset.yml` runs it before caching an archive and
before building from one. `ods` doesn't check the signature itself yet.

**The signature vouches for the bytes, not the date.** In 13 of the 98 releases from 2018-06-29
to 2026-09-25, the signature is dated differently from the TRUD release, from a day early
(2026-06-26 was signed on 2026-06-25) to more than five months late (2018-09-28 was signed on
2019-03-08). So a release's date always comes from TRUD: from its API or listing, or from a record
written once from TRUD's word, like an index row, a TRUD archive package, or an archive image's tag and
`fyi.ods.trud-release-date` annotation in `nhs-ods-xml`.

## How a release is checked

- **`ods trud pull`** checks the zip against the SHA-256 TRUD's API gives, and writes that hash,
  TRUD's word, into the TRUD archive package. [trud.md](./trud.md) has the flow.
- **`ods make`** checks the zip against the TRUD archive package, the release index row or TRUD's API before
  it builds, and embeds what they say in every Parquet file.
- **`ods pull`** names a file only once its SHA-256 matches the manifest the index names, then
  rebuilds the manifest from the files and refuses a release whose digest differs.
- **`ods use`, `ods cite` and `ods make release`** rebuild the manifest from the files on disk and
  compare its digest with the index row ([oci.md](./oci.md#the-manifest-is-a-function-of-the-files)).
- **`scripts/verify-trud-bundle.sh`** checks NHS's signature, above.

## The rules

- **Provenance is TRUD's word.** The source's date, hash and size come from TRUD's API or a release
  index row that recorded them, never computed from a file `ods` built.
- **The files say what they are.** Every fact about a release lives in its Parquet files. The
  manifest, the view and the citation are all derived from them.
- **Embed what's true forever.** A fact about the bytes, or an upstream's own URL, goes in. A
  location we control doesn't.
- **Build-run facts live in the attestation.** Machine names, runner IDs and timestamps belong in
  the signed SLSA attestation outside the dataset. Inside it, they'd stop rebuilds reproducing
  the same digests.


[datapackage]: https://datapackage.org/
