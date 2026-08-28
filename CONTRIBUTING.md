# Contributing to ods

How to build it, the principles behind it, and the sharp edges to avoid.

## Getting started

The quick path, pulling published data from github:

- `cargo test` - Run the tests 
- `cargo install --path .` build the bin and add to your path
- `ods pull` pull the latest data from a github release
- `ods find sedbergh` - find active nhs entities in Sedbergh

```console
# Run the tests
$ cargo test
...test result: ok. 77 passed; 0 failed

# build the bin and add to your path
$ cargo install --path .
...Finished `release` profile [optimized] target(s) in 22.43s

# pull the latest data from a github release
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


## What we're building

NHS ODS data is a snapshot of the data that tells us "what is the NHS" from a legal orgs and physical buildings point of view. It's interesting data published each month. But it's trapped in a gigabyte of custom XML which is hard to work with, and you have to request an NHS TRUD account before you are allowed to download it.

So `ods` compiles the source XML into Parquet you can query remotely with `duckdb`, pandas, R,
or anything else that reads open formats. Correctness first, then usability. The
aim is to demonstrate a better way to publish this data, and for it to be a joy to use while doing it.

Verifiable provenance for the data is essential so `ods` manages pulling and verifying the source data, ensuring the file hashes match those published by the NHS.

```text
TRUD publishes a SHA-256 for the release archive
    │
    ▼ ods trud pull        pull the data, verify the hash
    │
_provenance.json
    │
    ▼ ods make             deterministic projections of the data
    │
*.parquet + manifest
```

Anyone with a TRUD account can rebuild the release and get identical bytes.

Four different claims, bought by four different mechanisms:

| Claim | Meaning | Secured by |
| :--- | :--- | :--- |
| **Provenance** | what this derives from | TRUD's hash, `tool_git_sha`, `Cargo.lock` at that sha |
| **Integrity** | bytes unchanged since publication | `manifest.json`, hashes baked into `ods`, mirrors |
| **Authenticity** | published by this project | the Zenodo record and the git history |
| **Correctness** | the derivation is faithful | **someone re-running it** |

Hashes prove the bytes you got are the bytes we published. They cannot prove
we derived them correctly — every copy is served from one build. What proves
that is rebuilding it: `tool_git_sha` and the TRUD hash in `_provenance.json`
are there so you can, and `ods trud audit` compares the result. If you do and
we disagree, please open an issue.

**Correctness depends on TRUD staying reachable.** Rebuilding needs the source
archive, and only NHS England distributes it. If TRUD is withdrawn, nobody can
re-run the derivation and our source hashes become claims you'd have to take on
trust. We keep a copy of every source archive to garud against that possibility.

Three audiences, wanting different things:

| Audience | Wants | Path |
| :--- | :--- | :--- |
| **Query** | zero setup, something citable | `ods pull`, `ods find`, `ods cite` |
| **Verify** | to rebuild it and check | `ods trud pull`, `ods make`, `ods trud audit` |
| **Publish** | the monthly release to go out right | CI, `ods make`, release workflow |

Most trade-offs come down to those three. Query is the biggest group. Verify is
why anyone trusts the output.

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

**Say `unknown` when you don't know.** Provenance fields are namespaced by whose
fact they are: `trud_*` from the TRUD API, `publication_*` from the ODS XML
manifest, `tool_*` from `ods` itself. A missing value reads `unknown` and never
uses a fallback. avoid offering plausible wrong answers. Same reasoning applies
to swallowed errors. Always clearly state what went wrong and where possible offer
steps to retry or fix it.

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

Every change needs a test, and tests run offline.

## Release Process

We separate **tool releases** from **dataset releases**:

### 1. Tool Releases (`scripts/release-tool.sh`)
When shipping a new version of the `ods` binary:
- Bump `version` in `Cargo.toml`.
- Run `scripts/release-tool.sh`. It verifies the working tree is clean, runs `cargo test`, tags `v<version>`, and pushes to origin.
- GitHub Actions builds binaries for all targets and attaches them to the GitHub release.

### 2. Dataset Releases (`scripts/release-data.sh <date> <version>`)
When publishing a monthly dataset cut or republishing a fix:
1. Pull and compile the release:
   ```console
   $ ods trud pull <date>
   $ ods make
   ```
2. Build and verify the OCI bundle, checking preconditions:
   ```console
   $ ods make release --version <version>
   ```
   This performs structural checks, verifies git tags, and appends the release row to `data/releases.json`.
3. If an earlier release for `<date>` had errors, mark the old row `withdrawn` in `data/releases.json`.
4. Commit `data/releases.json`, create git tag `data/<date>_<version>`, and publish:
   ```console
   $ git add data/releases.json && git commit -m "release(data): <date> v<version>"
   $ git tag data/<date>_<version>
   $ ods publish
   ```
   This uploads layers and image manifests directly to registry mirrors over OCI transport.

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

**CLI output** `✓` success, `*` cached, `✖` error, progress lines, and status lines 
only for work actually done. Data to stdout, progress to stderr, so `ods pull --list > file` is useful. Always exit non-zero when the command couldn't do its job.

## Gotchas

- **Parquet columns are resolved by name at runtime** If you rename a column The code
  will compile but fail at runtime. You gotta grep for the string.
- **nullable columns is declared explicitly rather than determined from the current data** 
  A mostly-null column will fail at write time, not compile time.
- The primary role describes the register rather than the organisation
- Successions are many-to-many
- Legal dates are mostly empty, prefer operational dates
- The most recent release is not a superset of all previous releases. Some orgs have
  been removed from teh dataset over time, and info about previous years gets updated.

[docs/queries.md] has the full list, with the queries that show each one.

## Licensing

The code is [AGPL-3.0]. It requires source disclosure from anyone running a
modified version as a network service.

The ODS data is published by NHS England under the [Open Government Licence], and
the derived Parquet inherits it. Attribution should point to NHS England first.

We publish the derived Parquet, not the TRUD XML. The NHS TRUD service asks people
to register for access to the source data so they can contact people when the data
is corrected or updated.

The parquet tables are our main output and the provenance chain points to the TRUD
for anyone who wants the source.

Per the OGL licence: 
- Report errors upstream when you find them,
- Keep a record of which release you used. 

Our per release `_provenance.json` covers the second point, for every artefact `ods` makes.

## Where next?

- [docs/parquet.md] — schemas, naming rules, and the decisions they settle
- [docs/queries.md] — worked queries, single-release and across an archive

[docs/parquet.md]: ./docs/parquet.md
[docs/queries.md]: ./docs/queries.md
[role_names.json]: ./data/role_names.json
[agpl-3.0]: https://www.gnu.org/licenses/agpl-3.0.en.html
[open government licence]: https://isd.digital.nhs.uk/trud/users/authenticated/filters/0/licence/26
