# The ods cli

i imagine 3 types of user for ods:

1. i want to do run some queries. i just want ods find to work. at some point i come to rely on this tool, and i want to cite it in my work and i am greatful that it makes the provenance clear (and i'm unaware that it's fully hash stable projections from a trusted upstream NHS provided source, but ok that sounds great!)
2. i want to run queries, and i really care about offline first and verifying data provenance. I will get a TRUD api key and want to be able to create my own copies. Oh cool, they hash to the same hash as the published ones. My sceptical self is mildly reassured and a little impressed, so few bother with these details.
3. Me, the "official" publisher of these derived artefacts to the github repo. I want to produce the artefacts with the same commands i would inflict on users (dogfooding) even tho they will end up as lines in a CI workflow soon. I want every months data to be published as a release on github on the same day as the NHS TRUD publishes new (unweildy) source xml files.

my thinking for splitting `ods trud` cmds out is so that user 1 (by far the most numerous) doesn't have to think about them. and also that `ods pull` fetching derived parquet from github is different enough from "fetch the official xml from the NHS with an api key". Once we had that split it made sense to push other dataset ops to the trud namespace. For `diff` i imagine use publishing a change log or similar so folks can check it without having to run `ods` themsevlves, but the trustless crew can run it if they want.

The CLI serves three distinct users:

| Goal | Need | Commands |
|---|---|---|
| **Query** — "I just want to search NHS orgs" | Zero-setup search, citable provenance | `find`, `pull`, `cite` |
| **Verify** — "I want to build from source and check hashes" | TRUD API access, reproducible builds | `trud pull`, `make`, `trud audit`, `trud verify` |
| **Publish** — "I produce the monthly release artifacts" | Full pipeline, changelogs, CI integration | `trud pull`, `make`, `trud diff`, `trud audit` |


This is the target output for the cli

```
NHS ODS Data Tool

Usage: ods <COMMAND>

Commands:
  find  Search NHS organisations and sites
  pull  Download pre-built dataset releases
  cite  Show provenance metadata and academic citation
  trud  Build from official NHS source data (requires TRUD API key)
  make  Compile TRUD XML into Parquet files or Markdown
  help  Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

This is the target output of `ods cite`

```
Source
  NHS Digital Organisation Data Service (ODS), published by HSCIC via NHS TRUD.
  Publication date:   2026-07-28
  Publication seq:    #4700
  Publication type:   Full
  Source SHA-256:     8151248ddc290f3affdabae22d88e0bbd118947d948ab7bdd37e74088cfba933
Derived Sources
  Created by:         github.com/olizilla/ods v0.1.0 
  Format:             Apache Parquet
  orgs.parquet:       b47da8fab975428af8fd19226dc3d2e937d05a42d018e7ae44ec034ba1a171d5
  orgs_all.parquet:   8b8ec72be1493c689e8d468e4b1e2e0a8097eb5f697998d06ea10beaf0d7d1bd
  roles.parquet:      ede9c910fb237a8de56eb25824e0ea279bde991b2ec3445248e364c4cd983589
  rels.parquet:       64b88b2123e4c9bffd980fded013cfcd35c1aa7b2a056b5b27bde079c3004283
  successors.parquet: 83bfd672e6610c465bdb82cd2eda58aba4c1284972119e16471c1e56951809fd
How to Cite
  When citing the source data:
    HSCIC. (2026). NHS Organisation Data Service — Full Publication
    (2026-07-28, Seq #4700). NHS TRUD. https://isd.digital.nhs.uk/trud
  When citing this derived dataset:
    Evans, O. (2026). ods: NHS Organisation Data in Open Formats
    (v0.1.0) [Software]. https://github.com/olizilla/ods
  These Parquet files are deterministic projections of the official
  TRUD ODS XML. You can verify this by running `ods trud audit` or
  by rebuilding from source with `ods trud pull && ods make`.
```

