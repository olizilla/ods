# Contributing to ods

How to build `ods`, the principles behind it, and the sharp edges to avoid.

## Getting started

The quick path, pulling published data from ods.fyi:

- Install the `duckdb` CLI (v1.4.1+) to run SQL equivalence tests and query Parquet files
- `cargo test` - Run the tests 
- `cargo install --path .` build the bin and add to your path
- `ods pull` pull the latest release from ods.fyi
- `ods find sedbergh` - find active nhs entities in Sedbergh

```console
# Run the tests
$ cargo test
...test result: ok.

# build the bin and add to your path
$ cargo install --path .
...Finished `release` profile [optimized] target(s) in 22.43s

# pull the latest release from ods.fyi
$ ods pull

# find active nhs entities in Sedbergh
$ ods find --in sedbergh
┌───────────┬──────────────────────────────────────────┬──────────┬──────────────────────────────────┬───────┐
│ ODS Code  ┆ Name                                     ┆ Postcode ┆ Roles                            ┆ Class │
╞═══════════╪══════════════════════════════════════════╪══════════╪══════════════════════════════════╪═══════╡
│ 8GJ58     ┆ PARKER M JUNE (ACUPUNCURIST)             ┆ LA10 5AU ┆ Non-NHS Organisation             ┆ org   │
│ A82608    ┆ SEDBERGH MEDICAL PRACTICE                ┆ LA10 5DL ┆ GP Practice +1                   ┆ org   │
│ A82608001 ┆ DR LUMB W & PARTNER                      ┆ LA10 5QJ ┆ Branch Surgery                   ┆ site  │
│ D2E8H     ┆ AP SD THIRTEEN LIMITED                   ┆ LA10 5BL ┆ Pharmacy Headquarter             ┆ org   │
│ EE112233  ┆ SEDBERGH PRIMARY SCHOOL                  ┆ LA10 5AL ┆ School, Community School         ┆ org   │
```

### The longer path: register and pull source data from NHS TRUD

- Request a [TRUD account](https://isd.digital.nhs.uk/trud/users/guest/filters/0/account/form) (takes about 24hrs)
- Sign in to TRUD and subscribe to the "ODS XML Organisation Data" release.
- Find your [TRUD api key](https://isd.digital.nhs.uk/trud/users/authenticated/filters/0/account/manage)
- Set it as `TRUD_API_KEY` in your env.
- `ods trud pull` fetch the latest xml from the api.
- `ods make` build out the derived resources
- `ods find sedbergh` - find active nhs entities in Sedbergh

## Who is this for?

Three audiences, wanting different things: people who query, people who verify by rebuilding,
and the maintainer who publishes. [docs/cli.md](./docs/cli.md#who-this-is-for) describes each
and the commands they use. Most trade-offs come down to those three.

## What we're building

NHS ODS data is a snapshot of the data that tells us "what is the NHS" from a legal orgs and physical buildings point of view. It's interesting data published each month. But it's trapped in a gigabyte of custom XML which is hard to work with, and you have to request an NHS TRUD account before you are allowed to download it.

So `ods` compiles the source XML into Parquet you can query remotely with `duckdb`, pandas, R,
or anything else that reads open formats. Correctness first, then usability. The
aim is to demonstrate a better way to publish this data, and for it to be a joy to use while doing it.

Verifiable provenance for the data is essential so `ods` manages pulling and verifying the source data, ensuring the file hashes match those published by the NHS.

1. TRUD publishes a SHA-256 for the release archive
2. Run `ods trud pull` to fech the data and verify its hash
3. Run `ods make` to create deterministic projections of the data as parquet files

Anyone with a TRUD account can rebuild the release and get identical bytes. But we expect most folks won't have a TRUD account, and shouldn't need one to query the open data, so we publish those parquet files as content-addressed [OCI images](./docs/oci.md), and allow uses to fetch them via `ods`

1. TRUD publishes a SHA-256 for the release archive
2. We build the parquet on Github Actions with SLSA attestations for provenance.
3. We push those files to ods.fyi and ghcr.io
4. We announce them via https://ods.fyi/releases.json
5. User just runs `ods pull` to fech the data and verify its hash

The things we care about are: 

| Features | What? | Secured by |
| :--- | :--- | :--- |
| **Provenance** | what this derives from | TRUD's hash, the release index row's `tool_git_sha`, `Cargo.lock` at that sha |
| **Integrity** | bytes unchanged since publication | the manifest digest on the release index row, which the files rebuild, hashes baked into `ods`, mirrors |
| **Authenticity** | published by this project | in the git history (and later: Zenodo) |
| **Correctness** | the derivation is faithful | **someone re-running it** |

Hashes prove the bytes you got are the bytes we published. They cannot prove
we derived them correctly — every copy is served from one build. What proves
that is rebuilding it: the TRUD hash every Parquet file carries and the `tool_git_sha`
in the release index row are there so you can, and `ods trud audit` compares the result. 
We test for this in CI. If you find a release you can't reproduce the published hash for
please open an issue.

**Correctness depends on TRUD staying reachable.** Rebuilding needs the source
archive, and only NHS England distributes it. If TRUD is withdrawn, nobody can
re-run the derivation and our source hashes become claims you'd have to take on
trust. We keep a copy of every source archive to guard against that possibility
(and to be considerate and not hammer NHS bandwidth if we have to rebuild a release).

## How to decide things

These are the principles behind the decisions made so far

**Say what the source says.** ODS facts are carried verbatim. `publication_seq_num`
is spelled the way the XML spells it, and TRUD's uppercase hashes stay uppercase.

**Keep opinions separable.** Where we deviate, we write down why: role display names
are curated with typos fixed and abbreviations expanded, each substantive change
is documented in [role_names.json].

**Claim exactly what the mechanism delivers.** for example `ods pull` verifies 
checksums fetched from the same host as the data. That proves _integrity of transfer_, 
but can't tell you whether the host was compromised. Write that plainly.

**Say `unknown` when you don't know.** Aavoid offering plausible wrong answers.
Same reasoning applies to swallowed errors. Always clearly state what went wrong
and where possible offer steps to retry or fix it.

## What counts as done

Verify the artefacts, not just the test suite. Parser bugs can give sensible
looking row counts but corrupt key fields. We are creating reproducible projections
from stable monthly sources. Always compare the output to the input.

- Run the command. Query the Parquet with `duckdb`. Read the output you got.
- When you touch the parser or an export, check a known record field-by-field
  against the raw XML.
- Prove new checks can fail. Break the thing on purpose and watch it go red. A CI
  step guarded by a condition that's never true reports success having tested
  nothing.

A change to a guarantee in [docs/tests.md](./docs/tests.md) needs a test that
fails when that guarantee breaks, and the page gains or updates the guarantee
first. Tests run offline.

## Release process

We separate **tool releases** from **dataset releases**. A dataset release builds from a tool
release's tag, so the tool comes first. You need `gh`, `oras`, `jq`, `duckdb` and, to publish to
ods.fyi, `op` (1Password) and `rclone`.

### Tool releases (`scripts/release-tool.sh`)

When shipping a new version of the `ods` binary:

1. If the schema cut changes, bump the dataset version: `scripts/bump-version.sh dataset <x.y.z>`.
   It rewrites `DATASET_VERSION`, `data/datapackage.json` and the worker's expected keys.
2. Bump the tool: `scripts/bump-version.sh tool <x.y.z>`. A dataset bump needs a tool bump at least
   as large (patch, minor or major to match), so bump it even when the tool has no other change.
   `scripts/bump-version.sh` with no arguments prints both versions.
3. Add the `CHANGELOG.md` entry, newest first, as `## [<x.y.z>] - <date>`. Its first line names the
   dataset version the tool builds, e.g. `Builds dataset 0.2.0.`
4. Commit and push to `main`.
5. `scripts/release-tool.sh` is a dry run. It refuses on a dirty tree, a branch other than `main`,
   an existing tag, a missing changelog entry or a failing `cargo test`, and prints the release notes.
6. `scripts/release-tool.sh --publish` tags `v<version>`, pushes the tag and creates the GitHub
   release from that changelog entry. It doesn't build or attach binaries: there's no workflow for
   that yet.

A tag can never move: a repository ruleset forbids it. A fix to a released tool is a new patch
version, not a re-tag.

### Dataset releases

When publishing a monthly dataset cut, or rebuilding the back catalogue after a schema change.

**Before a package's first run.** A run never creates a dataset package on ghcr.io, so it has to
exist first. Once for each of `ods-data-rehearsal` and `ods-data`:

1. Create it by hand with a placeholder push:
   `oras push ghcr.io/olizilla/<repository>:init README.md`
2. On the package's page, under *Manage Actions access*, add `olizilla/ods` with the Write role.
3. Under *Package settings*, make it Public.
4. Wait until a stranger can read it. `scripts/ci-check-package-public.sh <repository>` checks
   that one can, and the workflow runs it first, so a package that isn't readable fails the run
   before it builds anything.

**The steps**

1. Rehearse, then release. `repository` defaults to `ods-data-rehearsal`: dispatch there first to
   try the whole run, from any ref (`main` included, so a CI fix can be tried before it's tagged):
   ```console
   $ gh workflow run build-dataset.yml --ref main -f dates=<date> -f repository=ods-data-rehearsal
   ```
   Its `ods make release --rehearsal` skips only the check that the tag points at the commit. Then
   dispatch from the tool tag `v<version>` with `ods-data` to publish for real: a release refuses
   to run from anywhere else.
   ```console
   $ gh workflow run build-dataset.yml --ref v<version> -f dates=<date> -f repository=ods-data
   ```
   Set `dates` to `latest` for a new TRUD release, to one or more space-separated `YYYY-MM-DD`
   dates, or to `all` to rebuild the back catalogue.
2. The workflow fetches any archive not yet cached in `ghcr.io/olizilla/nhs-ods-xml` from TRUD,
   checks NHS's signature, and caches it there. It then builds each date on `macos-15` and
   `ubuntu-24.04`, and refuses to go on if their manifest digests disagree. The Ubuntu build
   pushes the dataset to `ghcr.io`, pulls it back to re-verify it, and attests it. The run ends
   with a `candidate` artifact: `candidate.json`, which is `data/releases.json` with this run's
   rows added.
3. Download it, from the repo root, using the run's id (the number in the run's URL):
   ```console
   $ gh run download <run-id> -n candidate
   ```
   It lands as `candidate.json`, which `.gitignore` covers.
4. Copy the dataset's blobs and manifests to `ods.fyi`'s R2 bucket. `<dataset version>` is the
   dataset's, e.g. `0.2.0`, not the tool's. The first command is a dry run: it pulls the image
   from ghcr.io into a scratch directory, checks its manifest digest against `candidate.json`'s row
   and prints the bucket keys in upload order. The second uploads them, then fetches every key
   back from ods.fyi and checks each digest.
   ```console
   $ scripts/mirror-to-ods-fyi.sh <date> <dataset version> --index candidate.json
   $ scripts/mirror-to-ods-fyi.sh <date> <dataset version> --index candidate.json --publish
   ```
   The blobs are named by their digests, so publishing them announces nothing: only
   `releases.json` does that, and this script never uploads it. It never moves the `latest` tag
   either, which is what `https://ods.fyi/orgs.parquet` serves: that moves with `releases.json`,
   in the next step but one.
5. Try it before blessing it. From an empty directory *outside the repo*, pull with the candidate as
   the index. Its mirrors list ods.fyi first, so this reads what you just uploaded:
   ```console
   $ cd "$(mktemp -d)"          # in fish: cd (mktemp -d)
   $ ods pull --index <path to repo>/candidate.json
   $ ods find sedbergh
   $ ods cite
   ```
6. Bless it. Copy `candidate.json` over `data/releases.json`, commit that one file by name and
   push: git holds the canonical copy, so it goes first. Then `scripts/bless-release.sh` uploads
   it as ods.fyi's `releases.json` and points the `latest` tag at the newest release in it, in
   that order, so `https://ods.fyi/orgs.parquet` is never ahead of the index. It refuses an
   uncommitted index, or a newest release that isn't mirrored yet. Without `--publish` it's a
   dry run.
   ```console
   $ cp candidate.json data/releases.json
   $ git commit data/releases.json -m "data: release <date>_<dataset version>"
   $ git push
   $ scripts/bless-release.sh
   $ scripts/bless-release.sh --publish
   ```
7. Check it as a stranger would, from the repo root and then from an empty directory outside it:
   ```console
   $ curl -s https://ods.fyi/releases.json | cmp - data/releases.json
   $ duckdb -c "SELECT count(*) FROM 'https://ods.fyi/orgs.parquet'"
   $ cd "$(mktemp -d)"          # in fish: cd (mktemp -d)
   $ ods pull
   $ ods find sedbergh
   $ ods cite
   $ gh attestation verify oci://ghcr.io/olizilla/ods-data@<manifest digest> \
       -R olizilla/ods --signer-workflow olizilla/ods/.github/workflows/build-dataset.yml
   ```
   The manifest digest is on the release's row in `data/releases.json`.
8. Update the site, which shows the release's figures and terminal captures.
   ```console
   $ scripts/site-figures.sh ods_data/releases/<date>
   ```
   writes `site/src/data/release.json` from the release's Parquet files in your workspace
   (`ods pull` puts them there). Re-capture `site/src/data/terminal/cite.txt` from `ods cite`,
   keeping its `❯ ods cite` first line and its trailing `...`, and the two `find-gp-*.txt`
   captures from the commands on their `❯` lines. Commit, then deploy the worker, whose `deploy` script
   builds the site first:
   ```console
   $ cd worker && npm run deploy
   ```

**Credentials for R2.** `.r2.env` in the repo root (git-ignored, like every `*.env`) defines
rclone's `r2` remote as environment variables. The secret is a 1Password reference that `op run`
resolves, so no secret is on disk:

```
RCLONE_CONFIG_R2_TYPE=s3
RCLONE_CONFIG_R2_PROVIDER=Cloudflare
RCLONE_CONFIG_R2_ENDPOINT=https://<account id>.r2.cloudflarestorage.com
RCLONE_CONFIG_R2_REGION=auto
RCLONE_CONFIG_R2_ACCESS_KEY_ID=<access key id>
RCLONE_CONFIG_R2_SECRET_ACCESS_KEY="op://<vault>/<item>/<field>"
RCLONE_CONFIG_R2_ACL=private
RCLONE_CONFIG_R2_NO_CHECK_BUCKET=true
```

Check it lists the bucket:

```console
$ op run --env-file=.r2.env -- rclone lsf r2:ods-fyi --max-depth 1
```

If an earlier release for a date had errors, mark its dataset row `withdrawn` by editing
`data/releases.json` directly — no command writes that field yet.

If a source release itself has a known error, write its page, `docs/source-issues/<id>.md`, list
it in `docs/source-issues/README.md`, and add the ID to the release row's `source.issues` in
`data/releases.json` by hand. A release with no dataset yet gets a row with an empty `datasets`
list, its `source` from TRUD's word (the release's TRUD archive package, `trud/datapackage.json`). An ID, once added, stays.

Why it works this way is in [docs/release-index.md](./docs/release-index.md#how-a-dataset-gets-into-the-index): the
two runners, where each fact lives, the tag rules and the credentials split.

## Conventions

**Pre-1.0, so rename cleanly.** No compatibility shims, no legacy aliases, no
deprecated-but-still-works columns. Hashes change when the schema or encoding
changes, which is expected... update the fixtures and carry on.

That flips at the first published release. Once a dataset is citable it's
immutable, so a schema change breaks anyone who published numbers from it. Hence
the schema freeze happening before publication.

**Naming rules** are in [docs/parquet.md], along with the decisions they settle.
The `_code` suffix does two jobs, and a column name should be a phrase a
practitioner would say out loud.

## Updating dependencies

**An `arrow`, `parquet` or `zstd` change is a dataset change.** It gets its own branch and review,
and bumps the dataset version (and so the tool version, [D5]), even when the data comes out
identical.

Moving `parquet` from 53 to 60 wrote the same rows but different bytes, in every file. Holding
`created_by` at 53's value didn't help. There were four causes, and only the first can be pinned:

- Two `WriterProperties` defaults changed: page-header statistics went from always written to
  off, and statistics truncation from none to 64 bytes.
- `parquet` 60 needs a newer native `zstd` (1.5.6 → 1.5.7), which compresses the same pages
  differently.
- A newer `flatbuffers` serialises the `ARROW:schema` metadata differently.
- Footers and dictionaries changed in ways no setting reaches.

If one of these three changes through `Cargo.lock`, or any other crate changes the bytes,
`scripts/ci.sh`'s reproduce step fails. It rebuilds the newest dataset published at the current
dataset version and compares manifest digests.

## Gotchas

- **Parquet columns are resolved by name at runtime.** If you rename a column, the code
  will compile but fail at runtime. You gotta grep for the string.
- **Nullable columns are declared explicitly, rather than determined from the current data.**
  A mostly-null column will fail at write time, not compile time.
- The primary role describes the register rather than the organisation
- Successions are many-to-many
- Legal dates are mostly empty, prefer operational dates
- The most recent release is not a superset of all previous releases. Some orgs have
  been removed from the dataset over time, and info about previous years gets updated.

[docs/queries.md] has the full list, with the queries that show each one.

## Licensing

Everything written for this project — code, documentation, schemas and the role
vocabulary — is [MIT](./LICENSE). Copy a query, adopt a schema, reuse the code. If
you use `ods` in research, cite it from [CITATION.cff](./CITATION.cff).

Two things are NHS England's rather than ours, under the [Open Government Licence]:
the ODS data, which the derived Parquet inherits, and the test fixtures built from
it — `tests/fixtures/info/`, `trud_releases_response.json` and
`mock_hscorgrefdata.xml`. Attribute them to NHS England: *"Contains information from
NHS England, licensed under the current version of the Open Government Licence."*

We publish the derived Parquet, not the TRUD XML. The NHS TRUD service asks people
to register for access to the source data so they can contact people when the data
is corrected or updated.

The parquet tables are our main output and the provenance chain points to the TRUD
for anyone who wants the source.

Per the OGL licence:
- Report errors upstream when you find them,
- Keep a record of which release you used. (Baked in to our parquet files metadata)

## Where next?

- [docs/parquet.md] — schemas, naming rules, and the decisions they settle
- [docs/queries.md] — worked queries, single-release and across an archive
- [docs/provenance.md] — what each release says about where it came from, and whose word it is
- [docs/oci.md] — the OCI images: what's in a manifest, the tags, and fetching with stock tools
- [docs/datapackage.md] — `datapackage.json`, the TRUD archive package, and our Data Package profile

[docs/parquet.md]: ./docs/parquet.md
[docs/queries.md]: ./docs/queries.md
[docs/provenance.md]: ./docs/provenance.md
[docs/oci.md]: ./docs/oci.md
[docs/datapackage.md]: ./docs/datapackage.md
[D5]: ./docs/tests.md#the-data
[role_names.json]: ./data/role_names.json
[open government licence]: https://isd.digital.nhs.uk/trud/users/authenticated/filters/0/licence/26
