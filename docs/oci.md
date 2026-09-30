# OCI images

Every release is published as an [OCI image][oci-image]: the same content-addressed format
container registries serve, with Parquet files where a container would have filesystem layers.
Any OCI registry can host it, any OCI client can fetch it, and its digest names its exact bytes.

`ods` makes two kinds:

| | Datasets | Source bundles |
| :--- | :--- | :--- |
| **Repository** | `ods-data` | `nhs-ods-xml` |
| **`artifactType`** | `application/vnd.fyi.ods.dataset.v1` | `application/vnd.fyi.ods.source.v1` |
| **Holds** | a release's four Parquet files | a TRUD release exactly as NHS published it: the zip, NHS's checksum file, its PGP signature and NHS's public key |
| **Packed by** | `ods make oci` | `ods make oci --source`, pushed by `scripts/push-source-bundles.sh` |
| **Lives on** | ods.fyi and `ghcr.io/olizilla/ods-data` | `ghcr.io/olizilla/nhs-ods-xml`, private |

The dataset is what `ods pull` fetches. The source bundle is our copy of the upstream archive, kept
so a release can still be rebuilt if TRUD goes away ([CONTRIBUTING.md](../CONTRIBUTING.md) says
why), and never served to users.

## A dataset's manifest

The 2026-09-25 manifest, as `ods make oci` writes it and two independent builds reproduce. It is the published one: `curl -H 'Accept: application/vnd.oci.image.manifest.v1+json' https://ods.fyi/v2/ods-data/manifests/2026-09-25_0.2.0` returns these bytes, whose SHA-256 is the digest below.

```json
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.oci.image.manifest.v1+json",
  "artifactType": "application/vnd.fyi.ods.dataset.v1",
  "config": {
    "mediaType": "application/vnd.oci.empty.v1+json",
    "digest": "sha256:44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a",
    "size": 2
  },
  "layers": [
    {
      "mediaType": "application/vnd.apache.parquet",
      "digest": "sha256:2cd9c8924ab419845c5f1426997af8eabeac9c9579d6e09cbf13380d21826651",
      "size": 16393148,
      "annotations": { "org.opencontainers.image.title": "orgs.parquet" }
    },
    "… relationships.parquet, roles.parquet, successions.parquet …"
  ],
  "annotations": {
    "org.opencontainers.image.created": "2026-09-25T00:00:00Z",
    "org.opencontainers.image.description": "All the organisations and sites in the NHS Organisation Data Service, as queryable & verifiable Parquet files. Contains information from NHS England, licensed under the current version of the Open Government Licence.",
    "org.opencontainers.image.licenses": "OGL-UK-3.0",
    "org.opencontainers.image.source": "https://github.com/olizilla/ods",
    "org.opencontainers.image.title": "ods-data",
    "org.opencontainers.image.version": "2026-09-25_0.2.0"
  }
}
```

- **The config is the empty descriptor**, `application/vnd.oci.empty.v1+json`, the two bytes
  `{}`, as the image spec recommends for an artifact that isn't a container.
- **The layers are the Parquet files**, sorted by name, each titled with its file name.
- **The annotations are the image spec's own keys**, and nothing else:

| Annotation | From |
| :--- | :--- |
| `org.opencontainers.image.title` | the embedded object's `name` |
| `org.opencontainers.image.version` | its `version`, `<source release>_<dataset version>` |
| `org.opencontainers.image.description` | the dataset's one-sentence summary, then `licenses[0].attribution` |
| `org.opencontainers.image.licenses` | `licenses[0].name` |
| `org.opencontainers.image.created` | `sources[0].version`, TRUD's release date, as an RFC 3339 date-time at midnight UTC |
| `org.opencontainers.image.source` | the repository `ods` is built from |

There's no build time (`created` is the source's date, not the clock's) and nothing about the
tool that packed it, so any `ods` that builds the same zip at the same dataset version packs 
the same manifest, byte for byte. Which `ods` built a published dataset is on its [release index](./release-index.md)
row and in its attestation.

## A source bundle's manifest

The same shape: the empty config, and NHS's four files as layers, sorted by name and titled. Its
annotations name the TRUD release, and were frozen when the first bundle was pushed:

| Annotation | Value |
| :--- | :--- |
| `fyi.ods.trud-release-date` | TRUD's release date, from the TRUD archive package, `trud/datapackage.json`, not the date in the zip's name |
| `fyi.ods.trud-release-sha256` | the zip layer's digest, upper-case, as TRUD writes it: a convenience copy, not a second hash |
| `fyi.ods.attribution` | NHS England's attribution statement |
| `org.opencontainers.image.licenses` | `OGL-UK-3.0` |

Again no build time and nothing about the tool, so anyone holding NHS's four files packs the same
digest.

## The manifest is a function of the files

Nothing about a dataset's manifest is stored or chosen: it's computed from the Parquet files. So
`ods` verifies a release by rebuilding its manifest from the files on disk and comparing the digest
with the one its release index row names. `ods use`, `ods pull`, `ods cite`, `ods make release` and
`ods trud audit` all do it the same way, and a release directory needs no `oci/`:

```console
$ ods use 2026-09-25
* Release 2026-09-25 already active
  current → releases/2026-09-25
✓ reconstructed manifest sha256:0a43d08674948d678b2412b1303dd5bd27c3dcc52803f30f894d9a24edacaff0 matches the index for 2026-09-25 (0.2.0)
```

`ods make oci` writes an OCI image layout when you want one, to publish or to hand to another
tool:

```console
$ ods make oci -i releases/2026-09-25
* 4 layers, 28.2 MB
✓ oci/ written
  manifest sha256:0a43d08674948d678b2412b1303dd5bd27c3dcc52803f30f894d9a24edacaff0
✓ tags 2026-09-25, 2026-09-25_0.2.0
```

## Identity

A dataset id is its manifest digest. The digest covers every layer's digest, so it names the exact
bytes of all four files, and `ods cite` puts it in every citation.

As a [package URL][purl], which is how the `datapackage.json` view names the release in its `purl`
([datapackage.md](./datapackage.md)):

```text
pkg:oci/ods-data@sha256%3A0a43d08674948d678b2412b1303dd5bd27c3dcc52803f30f894d9a24edacaff0?repository_url=ods.fyi%2Fods-data
```

## Tags

Tags are the same on ghcr.io and ods.fyi:

- `<date>_<version>` names one dataset and never moves. A push that would move it is refused.
- `<date>` names the newest dataset built for that date, and moves.
- `latest` names the newest release on ods.fyi, and moves.

A source bundle has only `<date>`, and it never moves either: if NHS ever republished a date,
that would be news to report, not a tag to overwrite.

A tag doesn't bless anything. CI tags a dataset on ghcr.io when it pushes it, and the mirror
script tags it on ods.fyi, both before its index row is committed. `ods` finds datasets only
through the release index, and never reads a registry tag (`docs/tests.md` R3). The tags are
there for `oras`, `docker` and other OCI tools.

## Where the images live

- **ods.fyi** serves datasets from an R2 bucket through a Cloudflare worker (`worker/`), which
  speaks the OCI distribution API's read side. `ods` asks it first.
- **ghcr.io/olizilla/ods-data** holds the same datasets, pushed by CI. `ods` falls back to it.
- **ghcr.io/olizilla/nhs-ods-xml** holds the source bundles, privately. CI reads it to rebuild a
  release; users never need it.

The release index lists the dataset mirrors in order ([release-index.md](./release-index.md)).

## Fetching with stock tools

You don't need `ods` to fetch a release. These work against the published 2026-09-25 dataset:

```shell
# the four Parquet files, into a directory
oras pull ods.fyi/ods-data:2026-09-25_0.2.0 -o 2026-09-25

# the whole image, as an OCI layout
oras copy ods.fyi/ods-data:2026-09-25_0.2.0 --to-oci-layout ods-data:2026-09-25_0.2.0

# just the manifest
oras manifest fetch ods.fyi/ods-data:2026-09-25_0.2.0

# the manifest with curl
curl -H 'Accept: application/vnd.oci.image.manifest.v1+json' \
  https://ods.fyi/v2/ods-data/manifests/2026-09-25_0.2.0

# from ghcr.io, which wants a token even for a public image: an anonymous one will do
token=$(curl -s 'https://ghcr.io/token?scope=repository:olizilla/ods-data:pull' | jq -r .token)
curl -H "Authorization: Bearer $token" \
  -H 'Accept: application/vnd.oci.image.manifest.v1+json' \
  https://ghcr.io/v2/olizilla/ods-data/manifests/2026-09-25_0.2.0
```

The same works on a layout `ods make oci` wrote, which is how these were checked:

```console
$ oras pull --oci-layout releases/2026-09-25/oci:2026-09-25_0.2.0 -o oras-out
Downloaded  38dc432886c2 roles.parquet
Downloaded  3650dc5a2cd7 successions.parquet
Downloaded  29a8eb9b2dcf relationships.parquet
Downloaded  2cd9c8924ab4 orgs.parquet
Downloaded  0a43d0867494 application/vnd.oci.image.manifest.v1+json
Pulled [oci-layout] releases/2026-09-25/oci:2026-09-25_0.2.0
Digest: sha256:0a43d08674948d678b2412b1303dd5bd27c3dcc52803f30f894d9a24edacaff0
```

A directory holding just the four Parquet files is a release `ods` reads: the files carry their
own provenance, so nothing else needs to come with them.

```console
$ ods find --code RJZ -i oras-out
* Source: releases/2026-09-25/orgs.parquet
┌──────────┬──────────────────────────────────────────────┬──────────┬───────────────────────┬───────┐
│ ODS Code ┆ Name                                         ┆ Postcode ┆ Roles                 ┆ Class │
╞══════════╪══════════════════════════════════════════════╪══════════╪═══════════════════════╪═══════╡
│ RJZ      ┆ KING'S COLLEGE HOSPITAL NHS FOUNDATION TRUST ┆ SE5 9RS  ┆ NHS Trust, Hospice +1 ┆ org   │
├──────────┴──────────────────────────────────────────────┴──────────┴───────────────────────┴───────┤
│ 1 open                                                                 Use --all to include closed │
└────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

## Attestations

`build-dataset.yml` attests each dataset it pushes with [`actions/attest`][attest]: GitHub signs
the manifest digest, with the workflow, the ref and the commit that built it. Check one without
trusting the release index:

```bash
gh attestation verify oci://ghcr.io/olizilla/ods-data@<manifest_digest> \
  -R olizilla/ods --signer-workflow olizilla/ods/.github/workflows/build-dataset.yml
```

This checks GitHub's signature on the digest and prints the SLSA predicate: the workflow, the
ref (the tag `v<tool_version>`) and the commit it ran at. Reading `build-dataset.yml` at that
commit shows the rest: the `primary` job's *Compare with the witness* step fails before *Push*
unless the Ubuntu build's manifest digest equals the macOS witness's. So one attestation, on the
digest *Push* names, is enough. A reader verifies the digest was pushed by that workflow at that
commit, then reads the file to see that pushing only happens after the runners agreed
(`docs/tests.md` R9).

Later claims about a dataset (a DOI, a second builder's agreement, a review) belong beside it, not
in it: as artifacts whose `subject` is the dataset's manifest, found through the registry's
[referrers API][referrers]. They can be added at any time without changing the dataset's digest.

[oci-image]: https://github.com/opencontainers/image-spec
[purl]: https://github.com/package-url/purl-spec
[attest]: https://github.com/actions/attest
[referrers]: https://github.com/opencontainers/distribution-spec/blob/main/spec.md#listing-referrers
