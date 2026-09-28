# Provenance

Every Parquet file `ods` writes where it came from in it's metadata. 
A loose `orgs.parquet` is still citable and DuckDB can read it without `ods`:

```console
$ duckdb -c "SELECT decode(value) FROM parquet_kv_metadata('orgs.parquet') WHERE decode(key) = 'datapackage'"
{"name":"ods-data","version":"2026-09-25_0.1.0","licenses":[{"name":"OGL-UK-3.0","path":"https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/","title":"Open Government Licence v3.0","attribution":"Contains information from NHS England, licensed under the current version of the Open Government Licence."}],"contributors":[{"title":"NHS England","roles":["rightsHolder"]},{"title":"NHS TRUD","roles":["distributor"]}],"sources":[{"title":"NHS Organisation Data Service XML Data","version":"2026-09-25","path":"https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases","hash":"sha256:ca0fee7512f593ada1fa9b95bf1372b41911167da463a98fecf33adfd86697e5","bytes":38138574}]}
```

Everything `ods` says about a release comes from there. The file hashes in the OCI Image
mean it can't change without the release's digest changing too.

## The embedded object

One key-value metadata key, `datapackage`, holding compact JSON in [Data Package][datapackage]
vocabulary, with its keys in this order. All four files carry the same object:

```json
{
  "name": "ods-data",
  "version": "2026-09-25_0.1.0",
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

Files that don't carry the provenance are flagged by `ods` commands.

```console
$ ods use 2026-09-25
✖ The Parquet files in ods_data/releases/2026-09-25 don't carry the same provenance
  orgs.parquet           2026-09-25_0.1.1  (datapackage sha256:e6ee2e0ea7fd…)
  relationships.parquet  2026-09-25_0.1.0  (datapackage sha256:eb593d7b8e21…)
  roles.parquet          2026-09-25_0.1.0  (datapackage sha256:eb593d7b8e21…)
  successions.parquet    2026-09-25_0.1.0  (datapackage sha256:eb593d7b8e21…)
  A release's files all come from one build. Pull it again with `ods pull --force`, or rebuild it with `ods make`.
```

## The manifest

A release's OCI manifest is a function of its four Parquet files: the empty config
(`application/vnd.oci.empty.v1+json`), `artifactType` `application/vnd.fyi.ods.dataset.v1`, the
files as layers, titled, and annotations copied from the embedded object:

| Annotation | 2026-09-25 | From |
| :--- | :--- | :--- |
| `org.opencontainers.image.title` | `ods-data` | `name` |
| `org.opencontainers.image.version` | `2026-09-25_0.1.0` | `version` |
| `org.opencontainers.image.licenses` | `OGL-UK-3.0` | `licenses[0].name` |
| `org.opencontainers.image.created` | `2026-09-25T00:00:00Z` | `sources[0].version`, as an RFC 3339 date-time (midnight UTC) |
| `org.opencontainers.image.source` | `https://github.com/olizilla/ods` | the repo |
| `fyi.ods.attribution` | `Contains information from NHS England, …` | `licenses[0].attribution` |
| `fyi.ods.source.title` | `NHS Organisation Data Service XML Data` | `sources[0].title` |
| `fyi.ods.source.version` | `2026-09-25` | `sources[0].version` |
| `fyi.ods.source.hash` | `sha256:ca0fee75…97e5` | `sources[0].hash` |
| `fyi.ods.source.bytes` | `38138574` | `sources[0].bytes` |

So a release directory needs no `oci/`. `ods` verifies a release by rebuilding its manifest from
the files and comparing the digest with the release index row: `ods use`, `ods pull`, `ods cite`
and `ods make release` all do it the same way. `ods make oci` writes the OCI layout when you want
one, to publish. The 2026-09-25 manifest, the same from two separate pulls and builds:

```console
$ ods make oci -i releases/2026-09-25
* 4 layers, 28.2 MB
✓ oci/ written
  manifest sha256:bde37d59516a47b39627fe69459eda3ee4930fa9e75e569b4ab63ff63f74a08e
✓ tags 2026-09-25, 2026-09-25_0.1.0
```

## One vocabulary for every dataset

| Where | Names |
| :--- | :--- |
| The embedded object, `datapackage.json`, `trud/datapackage.json` | [Data Package][datapackage] property names: `name`, `version`, `licenses`, `contributors`, `sources`, `resources`, plus `licenses[].attribution` and the Source's `hash`, `bytes` and `_cache` from ods's profile |
| Manifest annotations with an OCI key | `org.opencontainers.image.title`, `.version`, `.licenses`, `.created`, `.source` |
| Manifest annotations without one | `fyi.ods.attribution`, `fyi.ods.source.title`, `fyi.ods.source.version`, `fyi.ods.source.hash`, `fyi.ods.source.bytes` |
| The release index | `trud_release_date`, `trud_release_sha256`, `trud_release_filesize_bytes`, `dataset_version` and the rest ([release-index.md](./release-index.md)) |

The profile is [`https://ods.fyi/schema/datapackage.v1.json`][profile]: JSON Schema draft-07, as
Data Package requires, `allOf` the published Data Package v2 package profile and a small mixin for
`licenses[].attribution` and the Source properties `hash` (`sha256:` and lower-case hex), `bytes`
and `_cache`. Nothing in it is specific to one dataset.

## `datapackage.json`: the view

`ods make` and `ods pull` always write a `datapackage.json` beside the Parquet files. It's the
readable explanation of a release directory, for people and for Frictionless tools. It's never
packed, and `ods` never reads a value from it: edit it and nothing `ods` says changes. It's the
embedded object plus `$schema`, `id`, `title` and `description`, the source's `_cache`, and one
resource per file with its `bytes`, `hash` and Table Schema. `ods make datapackage` writes it
again, and `-o -` prints it (here without the Table Schemas, and with the repeated parts elided):

```console
$ ods make datapackage -o - | jq 'del(.resources[].schema)'
{
  "$schema": "https://ods.fyi/schema/datapackage.v1.json",
  "id": "pkg:oci/ods-data@sha256%3Abde37d59516a47b39627fe69459eda3ee4930fa9e75e569b4ab63ff63f74a08e?repository_url=ods.fyi%2Fods-data",
  "name": "ods-data",
  "version": "2026-09-25_0.1.0",
  "title": "ods: NHS Organisation Data as verifiable Parquet files",
  "description": "All the organisations and sites in the NHS Organisation Data Service, as queryable & verifiable Parquet files. Deterministic projections of NHS England's ODS XML release on NHS TRUD, published by ods.fyi.",
  "licenses": [ "… as in the embedded object …" ],
  "contributors": [ "… as in the embedded object …" ],
  "sources": [
    {
      "title": "NHS Organisation Data Service XML Data",
      "version": "2026-09-25",
      "path": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
      "hash": "sha256:ca0fee7512f593ada1fa9b95bf1372b41911167da463a98fecf33adfd86697e5",
      "bytes": 38138574,
      "_cache": [
        "https://ghcr.io/v2/olizilla/nhs-ods-xml/blobs/sha256:ca0fee7512f593ada1fa9b95bf1372b41911167da463a98fecf33adfd86697e5"
      ]
    }
  ],
  "resources": [
    {
      "name": "orgs",
      "type": "table",
      "path": "orgs.parquet",
      "format": "parquet",
      "mediatype": "application/vnd.apache.parquet",
      "bytes": 16393155,
      "hash": "sha256:f9b65e9faf18f0b8652d0a093cb3c5e7012a57a3e004f9df16d57c34e2975373"
    },
    "… roles, relationships, successions …"
  ]
}
```

`id` is the release's `dataset_doi` when its index row has one and names this exact manifest,
else its [package URL][purl], built from the rebuilt manifest's digest. The view isn't part of
any digest, so it may name one. The same files and index give the same bytes, every time. A build
without provenance gets a view with no `id`, `version` or `sources`.

## `trud/datapackage.json`: the pull record

The TRUD zip can't carry our metadata, so `ods trud pull` describes the release beside it, in
`releases/<date>/trud/datapackage.json`, and writes nothing outside `trud/`. The archive's hash
and size are TRUD's word, from its API; the other three files' are hashed from what was downloaded:

```json
{
  "$schema": "https://ods.fyi/schema/datapackage.v1.json",
  "name": "nhs-ods-xml",
  "version": "2026-09-25",
  "title": "NHS Organisation Data Service XML Data",
  "homepage": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
  "licenses": [ "… as in the embedded object …" ],
  "contributors": [ "… as in the embedded object …" ],
  "resources": [
    { "name": "archive", "path": "hscorgrefdataxml_data_9.0.0_20260925000001.zip", "mediatype": "application/zip", "bytes": 38138574, "hash": "sha256:ca0fee7512f593ada1fa9b95bf1372b41911167da463a98fecf33adfd86697e5" },
    { "name": "checksum", "path": "trud_hscorgrefdataxml_data_9.0.0_20260925000001.xml", "mediatype": "application/xml", "bytes": 195, "hash": "sha256:00dec07290ed2aeda560c9eb74fa754328629e2cef13339900aeb8c9764c54cd" },
    { "name": "signature", "path": "trud_hscorgrefdataxml_data_9.0.0_20260925000001.xml.asc", "mediatype": "application/pgp-signature", "bytes": 488, "hash": "sha256:a78360f5bf2f6586b708270fa590f61df80118b420203b54288eeb0c2c89f48a" },
    { "name": "key", "path": "trud-public-key-2013-04-01.pgp", "mediatype": "application/pgp-keys", "bytes": 1736, "hash": "sha256:7a5f768367f8ebfbc9ab34d44e53f45e14c351949125d054d96e118a09323088" }
  ]
}
```

`ods make` reads it, and derives the embedded object's `sources[0]` from it, verbatim: `title`,
`version`, `path` from `homepage`, and `hash` and `bytes` from the archive resource. Its `name` is how `ods make` tells a source directory's record from a release's view. A
bare zip has no record, so `ods make <zip>` takes the same facts from the release index row, or
from TRUD's API when `TRUD_API_KEY` is set, and writes no `trud/`.

An older `ods` wrote `trud/_provenance.json` instead. Nothing reads it now. `ods make` in a
directory that still has one says what to run, and `ods trud pull <date>` replaces it:

```console
$ ods make
✖ releases/2026-09-25 holds trud/_provenance.json, the record an older ods wrote, and no trud/datapackage.json
  Write it from TRUD's listing, then build: ods trud pull 2026-09-25 --force && ods make
```

Parquet files an older `ods` built carry no `datapackage` key. `find`, `info` and `role` read
them with a warning, and `cite` refuses them. `ods trud pull <date> --force && ods make` rebuilds
them, and `ods pull --force <date>` fetches the published ones.

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
- **`ods make <zip> -o <dir> --force` builds it without provenance.** The files embed no
  `version` and no `sources`, the `trud_release_date` column holds the date in the zip's name,
  and the build warns after its block:
  ```console
  ! hscorgrefdataxml_data_9.0.0_20260925000001.zip isn't a TRUD release ods knows (SHA-256 CA0FEE75…97E5)
    Built without provenance. You can explore it with find, info and role, but not cite or publish it.
  ```
- **`ods trud pull --local-archive <zip>` refuses it:** `Build it outside the workspace with -o
  <dir>, or run ods pull for a newer release index.` It has no `--force` of its own.
- **`find`, `info` and `role` read it, with one warning:**
  ```console
  $ ods find --code RJZ -i force
  ! force has no provenance: it was built from an archive ods couldn't match to a TRUD release
  ```
- **`cite`, `make oci`, `make release` and `trud audit` refuse it:**
  ```console
  $ ods cite -i force
  ✖ force has no provenance: it was built from an archive ods couldn't match to a TRUD release
    To cite or publish it, get the archive through ods trud pull.
  ```

## NHS's signature

Alongside each archive, TRUD serves a checksum file naming the zip and its SHA-1, NHS England's
PGP signature of that file, and NHS's public key. `ods trud pull` downloads all three, and the
pull record names each with its hash. `scripts/verify-trud-bundle.sh` checks them: the key's
fingerprint is pinned in the release index (`trud_signing_key_fingerprints`), the signature is
good, and the signed SHA-1 is the zip's. `build-dataset.yml` runs it before caching an archive and
before building from one. `ods` doesn't check the signature itself yet.

**The signature vouches for the bytes, not the date.** In 13 of the 98 releases from 2018-06-29
to 2026-09-25, the signature is dated differently from the TRUD release, from a day early
(2026-06-26 was signed on 2026-06-25) to more than five months late (2018-09-28 was signed on
2019-03-08). So a release's date always comes from TRUD: from its API or listing, or from a record
written once from TRUD's word, like an index row, a pull record, or an archive image's tag and
`fyi.ods.trud-release-date` annotation in `nhs-ods-xml`.

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
[profile]: https://ods.fyi/schema/datapackage.v1.json
[purl]: https://github.com/package-url/purl-spec
[cyclonedx]: https://cyclonedx.org/
[cdx-schema]: https://github.com/CycloneDX/specification/tree/master/schema
