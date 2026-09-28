# Data Package descriptors

`ods` writes two [Data Package][datapackage] descriptors: a release's `datapackage.json`, which
describes its Parquet files, and `trud/datapackage.json`, which records the TRUD release they
were built from.

## `datapackage.json`

`ods make` and `ods pull` create a `datapackage.json` as a readable explanation of a release directory.

Here it is for 2026-09-25, without the Table Schemas:

```console
$ ods make datapackage -o - | jq 'del(.resources[].schema)'
```
```json
{
  "$schema": "https://ods.fyi/schema/ods-datapackage.v1.json",
  "id": "pkg:oci/ods-data@sha256%3Aace1c6babc5b4abb1faaa11fa7a63ca8c1615a1169d7fa21ec8ee256b6d0e74c?repository_url=ods.fyi%2Fods-data",
  "name": "ods-data",
  "version": "2026-09-25_0.1.0",
  "title": "ods: NHS Organisation Data as verifiable Parquet files",
  "description": "All the organisations and sites in the NHS Organisation Data Service, as queryable & verifiable Parquet files. Deterministic projections of NHS England's ODS XML release on NHS TRUD, published by ods.fyi.",
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
      "bytes": 16393148,
      "hash": "sha256:fe972999b2fcfe7497967e0c3f38c4a450aab08cfa5de79654d0bf3c1429b4b6"
    },
    "… roles, relationships, successions …"
  ]
}
```

| Field | Where it comes from |
| :--- | :--- |
| `$schema` | the profile, below |
| `id` | the release index row's `dataset_doi`, when the row names this exact manifest and has one; otherwise the release's [package URL][purl], from the manifest the files rebuild ([oci.md](./oci.md)). The view isn't part of any digest, so it may name one |
| `name`, `version`, `licenses`, `contributors` | the object the Parquet files carry |
| `title`, `description` | `ods`: the dataset's title and description |
| `sources` | the object the Parquet files carry, plus `_cache`: our own copy of the TRUD zip, in the `nhs-ods-xml` image on ghcr.io, addressed by the zip's SHA-256 |
| `resources[]` `name`, `type`, `path`, `format`, `mediatype`, `schema` | `ods`'s compiled schemas: the same tables, columns and types as [parquet.md](./parquet.md) and the committed `data/datapackage.json` |
| `resources[]` `bytes`, `hash` | the files on disk |

Files without provenance (a `--force` build, or files an older `ods` built) get a view of
`$schema`, `resources` and a `description` saying their source and terms are unknown.

## `trud/datapackage.json`: the upstream metadata

`ods trud pull` describes the source archive in `releases/<date>/trud/datapackage.json`.

```json
{
  "$schema": "https://ods.fyi/schema/ods-datapackage.v1.json",
  "name": "nhs-ods-xml",
  "version": "2026-09-25",
  "title": "NHS Organisation Data Service XML Data",
  "homepage": "https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases",
  "licenses": [ "… as in the view …" ],
  "contributors": [ "… as in the view …" ],
  "resources": [
    { "name": "archive", "path": "hscorgrefdataxml_data_9.0.0_20260925000001.zip", "mediatype": "application/zip", "bytes": 38138574, "hash": "sha256:ca0fee7512f593ada1fa9b95bf1372b41911167da463a98fecf33adfd86697e5" },
    { "name": "checksum", "path": "trud_hscorgrefdataxml_data_9.0.0_20260925000001.xml", "mediatype": "application/xml", "bytes": 195, "hash": "sha256:00dec07290ed2aeda560c9eb74fa754328629e2cef13339900aeb8c9764c54cd" },
    { "name": "signature", "path": "trud_hscorgrefdataxml_data_9.0.0_20260925000001.xml.asc", "mediatype": "application/pgp-signature", "bytes": 488, "hash": "sha256:a78360f5bf2f6586b708270fa590f61df80118b420203b54288eeb0c2c89f48a" },
    { "name": "key", "path": "trud-public-key-2013-04-01.pgp", "mediatype": "application/pgp-keys", "bytes": 1736, "hash": "sha256:7a5f768367f8ebfbc9ab34d44e53f45e14c351949125d054d96e118a09323088" }
  ]
}
```

| Field | Source |
| :--- | :--- |
| `name` | `ods`: `nhs-ods-xml`, the source's name. It's how `ods make` tells a pull record from a release's view |
| `version` | NHS TRUD release date |
| `title`, `homepage` | NHS TRUD |
| `licenses`, `contributors` | NHS England's terms, as TRUD publishes them |
| the `archive` resource's `bytes` and `hash` | NHS TRUD |
| the other resources' `bytes` and `hash` | hashed from the files `ods trud pull` downloaded |

`ods make` reads it, builds the zip its `archive` resource names, and checks that zip's SHA-256
and size against it. Use `ods trud pull <date> --force` to refetch or repair a damaged source archive.

## The profile

Both descriptors name [`https://ods.fyi/schema/ods-datapackage.v1.json`][profile] as their
`$schema`: version 1 of our extension to Data Package 2.0.

- **The published Data Package v2 package profile**, inlined verbatim from
  `datapackage.org/profiles/2.0/datapackage.json` rather than `$ref`-ed over the network. A
  published profile never changes, so a validator needs no network to check against it. It has
  one addition, the List Field, below.
- **A small mixin** with what `ods` adds: `licenses[].attribution`, and on a Source, `hash`
  (`sha256:` then lower-case hex), `bytes` (an integer) and `_cache` (a list of URLs). Nothing in
  it is specific to one dataset or one source.

The v2 profile already allows extra properties on a licence or a Source; the mixin is there so a
mistake like `"bytes": "big"` fails, where the published profile alone would let it through.

## List columns

`orgs` has four list columns, `role_codes`, `role_names`, `predecessor_codes` and
`successor_codes`, each a Parquet `LIST<VARCHAR>`. The view declares them as Table Schema's `list`
type, with `itemType: "string"`:

```json
{ "name": "role_codes", "type": "list", "itemType": "string", "constraints": { "required": true } }
```

That's ahead of the published spec. The v2.0 profile allows only its listed field types, and
`list` isn't one of them, so a descriptor using it fails there. Frictionless have the List Field in
the source dictionary on `main` (`tableSchemaFieldList` in `profiles/dictionary/schema.yaml`), and
a `v2.0.1` hotfix is planned to publish it ([datapackage#1089][dp-1089]).

We went ahead because `array` was wrong for these columns: an `array` is a JSON array in a cell,
and a tool that reads Parquet by its types, like frictionless-ts, rightly reports every row of a
`LIST` column as the wrong type. When 2.0.1 publishes, our profile inlines the published 2.0.1
profile verbatim instead. The Parquet files, their metadata and every digest are unaffected by
any of this, so it needs no dataset version bump.

## `licenses[].attribution`

The OGL lets you reuse the data on condition that you acknowledge the source with the
"attribution statement" the provider specifies. The attribution is a condition of the licence, so
it's a field on the licence, not a free-floating note. [Camera Trap Data Package][camtrap-dp], one
of the two extensions datapackage.org lists, set the precedent for extending `licenses` (its
`scope`).

The same sentence goes by other names elsewhere:

| Where | Its name there |
| :--- | :--- |
| schema.org | `creditText` |
| SPDX | the package's attribution text (`attributionTexts` in SPDX 2.3 JSON) |
| CycloneDX | the component's `copyright` (see below) |
| OCI | the second half of a dataset manifest's `org.opencontainers.image.description` ([oci.md](./oci.md)) |

`ods cite` prints it under the source's citation.

## Frictionless tools

As measured on 2026-09-28, with frictionless-ts 1.0.3 and frictionless-py 5.19.1:

- **frictionless-ts validates a release, files and all.** It reads `$schema` and validates
  against our profile, fetched over `http(s)` (it won't load a profile from a local path). Its
  Polars reader sees a Parquet `LIST` as a `list`, which now matches the declared type, and it
  checks every file's `bytes` and `hash` for real: the 2026-09-25 release validates in about a
  second. Against the published 2.0 profile, the view fails on the four `list` fields.
- **frictionless-py can't read the view.** It ignores `$schema` and checks field types against
  its own list, which has no `list`, so it stops at `field type "list" is not supported`. That
  holds after 2.0.1 publishes too, until frictionless-py adds the type. With the columns declared
  `array` it read the descriptor but not the data: it reads Parquet through pandas, which hands
  list cells over as NumPy arrays, so every list column read as a type error, and it skips
  counting and hashing Parquet files, so every declared `bytes` and `hash` failed. Those are bugs
  in its Parquet reader, not in the release. It needs `pandas` installed alongside the `parquet`
  extra ([#1773][fl-1773]), and loads each file whole ([#1203][fl-1203]).

## CycloneDX

Supply-chain tools that read [CycloneDX][cyclonedx] rather than Data Package can have it from the
view, with `jq`:

```console
$ jq -f view-to-cyclonedx.jq releases/2026-09-25/datapackage.json > bom.json
```

where `view-to-cyclonedx.jq` is:

```jq
# CycloneDX 1.6 from a release's datapackage.json view.
{
  bomFormat: "CycloneDX",
  specVersion: "1.6",
  version: 1,
  metadata: {
    component: ({
      type: "data",
      name: .name,
      version: .version,
      description: .title,
      licenses: [.licenses[] | {license: {name: .name, url: .path}}],
      copyright: .licenses[0].attribution,
      pedigree: {
        ancestors: [.sources[] | {
          type: "data",
          name: .title,
          version: .version,
          hashes: [{alg: "SHA-256", content: (.hash | sub("^sha256:"; ""))}],
          externalReferences: ([{type: "distribution", url: .path}]
            + [(._cache // [])[] | {type: "distribution", url: .}])
        }]
      }
    } + (if (.id // "" | startswith("pkg:")) then {purl: .id} else {} end))
  },
  components: [.resources[] | {
    type: "data",
    name: .path,
    hashes: [{alg: "SHA-256", content: (.hash | sub("^sha256:"; ""))}]
  }]
}
```

The dataset sits under `metadata.component`: CycloneDX 1.6 has no top-level `component`. The
licence is `license.name`, not `license.id`, because `OGL-UK-3.0` isn't in SPDX's list and `id` is
checked against it. The program needs a view with provenance: one without has no `licenses` to
read.

[datapackage]: https://datapackage.org/
[profile]: https://ods.fyi/schema/ods-datapackage.v1.json
[purl]: https://github.com/package-url/purl-spec
[camtrap-dp]: https://camtrap-dp.tdwg.org/
[dp-1089]: https://github.com/frictionlessdata/datapackage/issues/1089
[da3526c]: https://github.com/frictionlessdata/datapackage/tree/da3526c4ed19563620d6a5e33ea55db6234cc411
[fl-1773]: https://github.com/frictionlessdata/frictionless-py/issues/1773
[fl-1203]: https://github.com/frictionlessdata/frictionless-py/issues/1203
[cyclonedx]: https://cyclonedx.org/
