# The ods cli

i imagine 3 types of user for ods:

1. i want to do run some queries. i just want ods find to work. at some point i come to rely on this tool, and i want to cite it in my work and i am greatful that it makes the provenance clear (and i'm unaware that it's fully hash stable projections from a trusted upstream NHS provided source, but ok that sounds great!)
2. i want to run queries, and i really care about offline first and verifying data provenance. I will get a TRUD api key and want to be able to create my own copies. Oh cool, they hash to the same hash as the published ones. My sceptical self is mildly reassured and a little impressed, so few bother with these details.
3. Me, the "official" publisher of these derived artefacts to the github repo. I want to produce the artefacts with the same commands i would inflict on users (dogfooding) even tho they will end up as lines in a CI workflow soon. I want every months data to be published as a release on github on the same day as the NHS TRUD publishes new (unweildy) source xml files.

my thinking for splitting `ods trud` cmds out is so that user 1 (by far the most numerous) doesn't have to think about them. and also that `ods pull` fetching derived parquet from github is different enough from "fetch the official xml from the NHS with an api key". Once we had that split it made sense to push other dataset ops to the trud namespace. For `diff` i imagine use publishing a change log or similar so folks can check it without having to run `ods` themsevlves, but the trustless crew can run it if they want.

The CLI serves three distinct users:

| Goal | Need | Commands |
|---|---|---|
| **Query** — "I just want to search NHS orgs" | Zero-setup search, citable provenance | `find`, `info`, `pull`, `cite` |
| **Verify** — "I want to build from source and check hashes" | TRUD API access, reproducible builds | `trud pull`, `make`, `trud audit`, `trud verify` |
| **Publish** — "I produce the monthly release artifacts" | Full pipeline, changelogs, CI integration | `trud pull`, `make`, `trud diff`, `trud audit` |


This is the target output for the cli

```
NHS ODS Data Tool

Usage: ods <COMMAND>

Commands:
  find  Search NHS organisations and sites
  info  Show full details for a single organisation by ODS code
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
  NHS England Organisation Data Service (ODS), published via NHS TRUD.
  Publication date:   2026-07-28
  Publication seq:    4700
  Publication type:   Full
  Release file:       hscorgrefdataxml_data_7.0.0_20260731000001.zip
  Release SHA-256:    8151248DDC290F3AFFDABAE22D88E0BBD118947D948AB7BDD37E74088CFBA933

Derived Sources
  Created by:         github.com/olizilla/ods v0.1.0 
  Format:             Apache Parquet
  orgs.parquet:       B47DA8FAB975428AF8FD19226DC3D2E937D05A42D018E7AE44EC034BA1A171D5
  orgs_all.parquet:   8B8EC72BE1493C689E8D468E4B1E2E0A8097EB5F697998D06EA10BEAF0D7D1BD
  org_roles.parquet:  F89C4B7D21A639B0E12356A8F83E71A12C59D960815E715B802FA834C19842A1
  roles.parquet:      EDE9C910FB237A8DE56EB25824E0EA279BDE991B2EC3445248E364CD983589
  rels.parquet:       64B88B2123E4C9BFFD980FDED013CFCD35C1AA7B2A056B5B27BDE079C3004283
  successors.parquet: 83BFD672E6610C465BDB82CD2EDA58ABA4C1284972119E16471C1E56951809FD

How to Cite
  When citing the source data:
    NHS England. (2026). Organisation Data Service: Full publication
    (2026-07-28, Seq 4700). NHS TRUD. https://isd.digital.nhs.uk/trud

  When citing the derived sources:
    Evans, O. (2026). ods: NHS Organisation Data in Open Formats
    (v0.1.0) [Software]. https://github.com/olizilla/ods

  These Parquet files are deterministic projections of the official
  TRUD ODS XML. You can verify this by running `ods trud audit` or
  by rebuilding from source with `ods trud pull && ods make`.
```

### JSON output is not yet stable

`--format json` is intended for scripts and agents, but its shape is **not
versioned**. Field names may change between `ods` releases without notice.

The Parquet schema has `dataset_parquet_schema_version` and a
`datapackage.json` contract; the CLI's JSON output has no equivalent yet. If you
are building something durable, read the Parquet files directly — they are the
stable interface, and `datapackage.json` describes them.

### Reproducibility Guarantee

Parquet projections are byte-identical for a given commit. Encoding parameters (ZSTD Level 3, max row group size 64,000) and tool metadata are embedded in the dataset metadata and `_provenance.json` (`tool_git_sha`, `tool_version`, `dataset_parquet_schema_version`, `dataset_file_sha256`).

