# Why `ods`?

> The NHS publishes a list of every organisation it works with, every month, for free. 
> It's brilliant. It's also 834MB of XML in a zip behind a login. This is what it could look like instead.

`ods` turns the NHS Organisation Data Service release into four Parquet files you can query with SQL, rebuild yourself to check, and fetch from any OCI registry by hash. [ods.fyi] and ghcr.io host them.

```console
$ ods pull
  2026-09-25    ████████████████████  28MB   4 files  from ods.fyi  in 3.9s
  dataset       ods-data/2026-09-25_0.2.0
  verified      sha256 from releases.json
  linked        current → releases/2026-09-25

$ ods find --gp --in SW9
* Source: releases/2026-09-25/orgs.parquet
* --gp: RO76, RO227, RO315 — GP Practice, Scottish GP Practice, Northern Ireland GP Practice
┌──────────┬──────────────────────────────┬──────────┬────────────────┬───────┬──────────┐
│ ODS Code ┆ Name                         ┆ Postcode ┆ Roles          ┆ Class ┆ Matched  │
╞══════════╪══════════════════════════════╪══════════╪════════════════╪═══════╪══════════╡
│ G85028   ┆ STOCKWELL GROUP PRACTICE     ┆ SW9 9TJ  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ G85054   ┆ LAMBETH WALK GROUP PRACTICE  ┆ SW9 6AF  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ G85073   ┆ VASSALL MEDICAL CENTRE       ┆ SW9 6NA  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ G85100   ┆ BECKETT HOUSE PRACTICE       ┆ SW9 9DL  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ G85135   ┆ MINET GREEN HEALTH PRACTICE  ┆ SW9 6AF  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ G85695   ┆ AKERMAN MEDICAL PRACTICE     ┆ SW9 6AF  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ Y00020   ┆ THE GRANTHAM PRACTICE        ┆ SW9 9BH  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ Y03063   ┆ HETHERINGTON AT THE PAVILION ┆ SW9 8DJ  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ Y05161   ┆ FIVEWAYS PCN EA HUB          ┆ SW9 6AF  ┆ GP Practice +1 ┆ org   ┆ postcode │
│ Y05163   ┆ LARC CLINIC (LA)             ┆ SW9 8DJ  ┆ GP Practice +1 ┆ org   ┆ postcode │
├──────────┴──────────────────────────────┴──────────┴────────────────┴───────┴──────────┤
│ 10 open                                                    Use --all to include closed │
└────────────────────────────────────────────────────────────────────────────────────────┘
```

No API key or user account. Every organisation and site the NHS has a code for, on your laptop, in seconds.

Or skip the install and point `duckdb` at the file over https:

```console
$ duckdb -box -c "FROM 'https://ods.fyi/orgs.parquet'
SELECT count(*) AS orgs, count(*) FILTER (WHERE status='active') AS active, max(publication_date) AS as_of"
┌────────┬────────┬────────────┐
│  orgs  │ active │   as_of    │
├────────┼────────┼────────────┤
│ 371569 │ 217091 │ 2026-09-21 │
└────────┴────────┴────────────┘
```

## What is the NHS Organisation Data Service?

The [Organisation Data Service][ods-nhs] is the register of everyone who exchanges data with the NHS. GP practices, trusts, pharmacies all the way to prisons, schools and local councils. Each one gets an ODS code, and many other NHS datasets use those codes to say who did what.

NHS England publishes it monthly on their [Technology Reference Update Distribution website (TRUD)][trud] as two massive XML files in a zip, and provides every snapshot back to 2018. It's open data under the [Open Government Licence][OGL]. To download it you register for a TRUD account, subscribe to the item, and wait for approval. Then you write a parser for 834MB of XML, or maybe even some XPath.

In practice people don't. They export a CSV from the [ODS portal][ods-portal] website for the codes they need. It works (_it is one of the better ones!_), but the provenance is gone the moment the download finishes.

CSVs and spreadsheets are mutable. Even the most careful people have cats near keyboards. Changes creep in _(I've wrestled with that on [dpimap.org] datasets)_. Then when someone does come to check the numbers some months later, they depend on a website still being online, with the same features, returning the same bytes for the same query.

## What are we aiming for?

**Open data should be a joy to use.** The ODS data is excellent. A well-documented [data model][ods-model] and 20 years of history. And XML is a flexible open format, good for modelling hierarchies. But that flexibility also makes querying harder. XPath or a custom parser is a high bar. Parquet is a more constrained open format. It's designed to be queried. `duckdb`, pandas, polars, R and Spark all read it, and over http too. 834MB of XML becomes 28MB of Parquet and queries are SQL.

**Open data must be reproducible.** `ods make` is deterministic. Give it the same TRUD zip and it writes the same bytes, on Linux or macOS. CI builds every release twice, on two operating systems, and refuses to publish unless both runners produce the same manifest digest. Anyone with a TRUD account can run `ods trud pull && ods make` and get the same hash. The hash is the claim. Reproducing it is the proof.

**Open data must stay available.** Each release is packed as an [OCI image][oci] and stored in standard container registries. That's the same open format that Docker and Homebrew use. The parquet files become layers addressed by the sha256 hash of their bytes. `ods pull` fetches the files by their hash from ods.fyi and falls back to ghcr.io. You can only ever get the exact file you asked for, and multiple mirrors can co-host the data. It also catches tampering and the odd solar-flare-induced bit flip, which are rare, but far more common than you'd think.

**Open data must credit its sources.** Every file has its provenance info baked in: which TRUD release it came from, that zip's SHA-256 as TRUD published it, the licence and the attribution. A loose `orgs.parquet` on a disk in five years can still be traced to its origin.

It's all wrapped with care in good UX, so the tool makes doing the right thing the easy thing. Nothing locks you in, or creates a dependency; `ods find --sql` shows you the SQL it would have run. `ods find` will build up a collection of baked-in queries that deal with the grain of the data as it is.

I'm calling it **data orthotics**: we cannot change the shape of our sources, but we can wrap them in a supporting tool that makes them easier to use naturally.

## How

The flow, from server farm to tables:

- `ods trud pull` fetches the zip from TRUD's API and checks it against the SHA-256 TRUD publishes, verifying the source.
- `ods make` streams the XML files from the zip, parses them, and writes four tables as parquet files: `orgs`, `roles`, `relationships` and `successions`. Compression and metadata are pinned so the output is byte-identical wherever it's built.
- Each file has metadata baked in holding the version, licence and source hash in the open [Data Package][datapackage] vocabulary. Again, care is taken to exclude build info or timestamps to preserve that determinism.

The republisher ([@olizilla]) runs those steps for you, in CI. As a researcher you don't need to deal with any of that. It happens in the background, and it is publicly auditable when you need it.

## Trust or verify

Releases are announced via `releases.json`. That's the trust anchor. It lists every release by its hash. The git repo is the record of what happened to the `releases.json`. From that, `ods pull` always checks you got the correct bytes for a release.

I don't sign releases. Why trust me with a magic key pair? Who even am [I](https://oli.zilla.org.uk/)? Releases can all be verified at any time by rebuilding them. It takes 6s per dataset on my 4yr old laptop. Rebuilding does need the source zip and a free TRUD account is a low bar. If you ever can't reproduce a published digest, that's a bug. Please [tell me][issues]!

CI does sign the public builds that create and push the dataset to the OCI registries. The workflow steps and the source tree at that time are [attested] so you can replay exactly how the data sausage was made at any point.

## What's in the data

All of this is the 2026-09-25 release. See [queries.md] for more.

A lot of ODS data isn't what you'd picture as the NHS. Schools, prisons and 1 Welsh Assembly are there too.

```console
$ ods role
* Source: releases/2026-09-25/orgs.parquet
┌───────┬─────────────────────────────────────────────────────────┬─────────┐
│ Code  ┆ Name                                                    ┆ Holders │
╞═══════╪═════════════════════════════════════════════════════════╪═════════╡
│ RO198 ┆ NHS Trust Site                                          ┆ 35,064  │
│ RO101 ┆ Social Care Site                                        ┆ 31,764  │
│ RO221 ┆ School                                                  ┆ 25,163  │
...
│ RO182 ┆ Pharmacy                                                ┆ 11,171  │
│ RO110 ┆ General Dental Practice                                 ┆ 9,795   │
│ RO76  ┆ GP Practice                                             ┆ 7,565   │
...
│ RO175 ┆ Prison                                                  ┆ 96      │
│ RO87  ┆ Walk-in Centre                                          ┆ 94      │
│ RO200 ┆ Welsh Assembly                                          ┆ 1       │
```


An `active` status on a row nearly always means open. Except when it doesn't. 4,877 organisations are marked active with a legal end date already passed, and 3,503 of those ended more than two years ago. NHS keeps a code open while systems migrate off it.

Organisations come back from the dead. And deletions happen: codes present in one release can be missing from the next. So the complete history is the union across all the releases, and the full back catalogue matters. With every release local, you'll be able to query the entire history in a second: closures, renames, reparenting, and who reported to whom and when. GP practices have barely moved in twenty years, while the layers above them reorganise regularly. With all 98 releases on disk you can see it.

That back catalogue will be published once I get some feedback on the shape of the current `2026-09-25` release.

## Where next

The pattern isn't specific to ODS. Any open dataset that's published as a file can be projected to Parquet deterministically, packed as an OCI image, and mirrored anywhere. ONS postcodes, the index of multiple deprivation, the NHS tech register. Each one becomes a reproducible image with its provenance baked in, and then you can join across them with wild abandon, instead of spending the afternoon pulling six sources into a spreadsheet by hand.

The ask to NHS England is small. Keep publishing the XML. Publish the hash, as you already do. And consider publishing the Parquet too, or signposting folks here!

## Kick the tyres

Install `ods` via [homebrew]. It pulls the binary from an OCI image on ghcr.io by its hash too!

```shell
# install
brew install olizilla/tap/ods

# fetch the latest release, verified against the index
ods pull

# search by name
ods find barts

# search by location
ods find --in SW9

# --gp is shorthand for the 3 different GP roles.
ods find --gp

# see more info by code
ods info R1H --all

# get the APA or bibtex screed
ods cite

# rebuild from source and compare the digest.
TRUD_API_KEY=<Your TRUD API key here> \
ods trud pull 2026-09-25 && ods make && ods cite
```

## Errors

Building this surfaced errors in the source. Per Trellick's [law of data maintenance] _"data is maintained to the level of detail shown by its most popular visualization"_, so I expect more will be found.

The data is NHS England's and the [TRUD terms] state that all errors should be reported. The plan is to report them in batches to avoid wasting NHS time.

Critical issues where the `ods` parser has to make a decision are tracked in [source-issues]. If you find more, please do open an [issue][issues].

## Credits

`ods` is inspired by [Marcus Baw][@pacharanero]'s [`sct`][sct] tool, which radically opens up the SNOMED CT data on similar principles. [@andrew] showed me the path of "use OCI for everything". It's a good enough content-addressing protocol, and critically it already has mass adoption through Docker and [Homebrew].

Big thanks of course to [David Eaves] and [Richard Allan] for nudging me on this path.

---

ods.fyi is an independent project. It is not affiliated with or endorsed by the NHS, but does aspire to be adopted there, either as a project or a set of ideas.



[ods.fyi]: https://ods.fyi
[ods-nhs]: https://digital.nhs.uk/services/organisation-data-service
[ods-portal]: https://www.odsdatasearchandexport.nhs.uk/
[dpimap.org]: https://dpimap.org/
[ods-model]: https://www.odsdatasearchandexport.nhs.uk/referenceDataCatalogue/ODS-Data-Model_571324843.html
[trud]: https://isd.digital.nhs.uk/trud/users/guest/filters/0/categories/5/items/341/releases
[TRUD terms]: https://isd.digital.nhs.uk/trud/users/guest/filters/0/licence/26
[OGL]: https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/
[oci]: https://opencontainers.org/
[datapackage]: https://datapackage.org/
[queries.md]: ./queries.md
[source-issues]: ./source-issues/README.md
[issues]: https://github.com/olizilla/ods/issues
[law of data maintenance]: https://trellick.work/writing/law-of-data-maintenance/
[sct]: https://github.com/pacharanero/sct
[@pacharanero]: https://github.com/pacharanero
[@andrew]: https://github.com/andrew
[homebrew]: https://github.com/homebrew
[@olizilla]: https://github.com/olizilla
[attested]: https://github.com/actions/attest#actionsattest
[David Eaves]: https://bsky.app/profile/eaves.ca
[Richard Allan]: https://ricallan.uk/
