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
$ ods find sedbergh
| ODS Code   | Name                                     | Postcode  | Category             | Class
|------------|------------------------------------------|-----------|----------------------|-----
| 8GJ58      | PARKER M JUNE (ACUPUNCURIST)             | LA10 5AU  | Non-NHS Organisation | org
| A82608     | SEDBERGH MEDICAL PRACTICE                | LA10 5DL  | GP Practice          | org
| A82608001  | DR LUMB W & PARTNER                      | LA10 5QJ  | Branch Surgery       | site
| EE112451   | SEDBERGH SCHOOL                          | LA10 5RY  | School               | org
| FLG02      | ALLIED PHARMACY SEDBERGH                 | LA10 5BL  | Pharmacy             | org
| RNN88      | SEDBERGH HEALTH CENTRE                   | LA10 5RX  | NHS Trust Site       | site
```

_Real output `SEDBURGH MEDICAL CENTRE` is misspelled in the source data. please open issues for data errors you find so we can report them in batches back to the NHS._

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
*.parquet + SHA256SUMS
```

Anyone with a TRUD account can rebuild the release and get identical bytes.

Four different claims, bought by four different mechanisms:

| Claim | Meaning | Secured by |
| :--- | :--- | :--- |
| **Provenance** | what this derives from | TRUD's hash, `tool_git_sha`, `Cargo.lock` at that sha |
| **Integrity** | bytes unchanged since publication | `SHA256SUMS`, hashes baked into `ods`, mirrors |
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
Where we deviate, we write down why: role display names are curated with typos
fixed and abbreviations expanded, each substantive change justified in
[role_names.json].

**Keep opinions separable.** `orgs.category` is a judgement call, because the ODS
`primary role` describes GP practices as `RO177 Prescribing Cost Centre`, captures
the administrative register that they are part of but is unhelpful for everyone else.
So the `category` column is an opinionated extension to the data. The rules that
determine its values live in [category_rules.json], shipped with the release and
hashed with everything else, with a `why` on each rule. If you disagree with it
you can edit it and recompute from the same inputs.
The bar for a rule is high: only where the primary role actively misleads about
the kind of thing. Taxonomic details belong in the `roles` column.

**Claim exactly what the mechanism delivers.** for example `ods pull` verifies 
checksums fetched from the same host as the data. That proves _integrity of transfer_, 
but can't tell you whether the host was compromised. Write that plainly.

**Say `unknown` when you don't know.** Provenance fields are namespaced by whose
fact they are: `trud_*` from the TRUD API, `publication_*` from the ODS XML
manifest, `tool_*` from `ods` itself. A missing value reads `unknown` and never
uses a fallback. avoid offering plausible wrong answers. Same reasoning applies
to swallowed errors. Always clearly state what went wrong and where possible offer
steps to retry or fix it. 

**Write down what you rejected.** Half the value in these docs is the options we
dropped, and why. Prescribing cost centre subtypes, sigstore signing,
`primary_role_code`, renaming successors to bare service names. Without the
reasoning, the next reader re-derives the same argument and eventually "fixes" a
deliberate choice.

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

- **Parquet columns are resolved by name at runtime** via `schema.index_of("...")`.
  A rename compiles clean and fails when it runs. Grep for the string.
- **Arrow nullability is declared explicitly.** A mostly-null column declared
  non-nullable fails at write time, not compile time.
- **Run the docs.** README examples have gone stale twice when the role vocabulary
  changed underneath them. If you touch a documented interface, execute the
  examples.

## The source data has issues

The primary role describes the register rather than the organisation, successions
are many-to-many, legal dates are mostly empty, and a snapshot taken today won't
reproduce one taken in 2019. [docs/queries.md] has the list, with the queries
that show each one.

## Licensing

The code is [AGPL-3.0]. It requires source disclosure from anyone running a
modified version as a network service.

The ODS data is published by NHS England under the [Open Government Licence], and
the derived Parquet inherits it. Attribution to NHS England is important.

We publish the derived Parquet and not the raw TRUD XML. That's a choice. OGL
may permit mirroring it, but NHS England use registration to reach
people when data is corrected and to collect error reports, and mirroring raw
files would skip that. The parquet is the product and the provenance chain points
at TRUD for anyone who wants the source.

Per the OGL licence: 
- report errors upstream when you find them,
- keep a record of which release you used. 

`_provenance.json` covers the second automatically, for every artefact `ods` makes.

## Where decisions live

- [docs/parquet.md] — schemas, naming rules, and the decisions they settle
- [docs/queries.md] — worked queries, single-release and across an archive
- [category_rules.json] — the classification opinion, with reasons, including
  removed rules


[docs/parquet.md]: ./docs/parquet.md
[docs/queries.md]: ./docs/queries.md
[category_rules.json]: ./data/category_rules.json
[role_names.json]: ./data/role_names.json
[agpl-3.0]: https://www.gnu.org/licenses/agpl-3.0.en.html
[open government licence]: https://isd.digital.nhs.uk/trud/users/authenticated/filters/0/licence/26
