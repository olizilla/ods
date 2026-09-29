# The ods cli

## Who this is for

Three types of user:

1. I want to do run some queries. i just want ods find to work. at some point i come to rely on this tool, and i want to cite it in my work and i am grateful that it makes the provenance clear (and i'm unaware that it's fully hash stable projections from a trusted upstream NHS provided source, but ok that sounds great!)
2. I want to run queries, and i really care about offline first and verifying data provenance. I will get a TRUD api key and want to be able to create my own copies. Oh cool, they hash to the same hash as the published ones. My sceptical self is mildly reassured and a little impressed, so few bother with these details.
3. The maintainer, publishing the derived artefacts. They want to produce the artefacts with the same commands users run (dogfooding), even though those end up as lines in a CI workflow, and every month's data published to ods.fyi on the day NHS TRUD publishes new source XML.

My thinking for splitting `ods trud` cmds out is so that user 1 (by far the most numerous) doesn't have to think about them. and also that `ods pull` fetching derived parquet from ods.fyi is different enough from "fetch the official xml from the NHS with an api key". Once we had that split it made sense to push other dataset ops to the trud namespace. For `diff` i imagine us publishing a change log or similar so folks can check it without having to run `ods` themselves, but the trustless crew can run it if they want.

The CLI serves three distinct users:

| Goal | Need | Commands |
|---|---|---|
| **Query** — "I just want to search NHS orgs" | Zero-setup search, citable provenance | `find`, `info`, `role`, `pull`, `cite` |
| **Verify** — "I want to build from source and check hashes" | TRUD API access, reproducible builds | `trud list`, `trud pull`, `make`, `trud audit`, `trud verify` |
| **Publish** — "I produce the monthly release artefacts" | Full pipeline, changelogs, CI integration | `trud list`, `trud pull`, `make`, `trud diff`, `trud audit` |

## Usage

Pull the latest parquet files with `ods pull`. Find NHS legal orgs and physical sites with `ods find`. Pass their ods code to `ods info <code>` to see all the details.

```
Fetch, query, cite and reproduce NHS ODS data as verifiable Parquet

Usage: ods <COMMAND>

Commands:
  find   Search NHS organisations and sites
  info   Show full details for a single organisation by ODS code
  role   Search and list role codes and names with holder counts
  pull   Download pre-built dataset releases
  cite   Show provenance metadata and academic citation
  trud   Build from official NHS source data (requires TRUD API key)
  make   Compile TRUD XML into Parquet tables
  use    Pin an active dataset release
  help   Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version
```

### JSON output

`--format json` is intended for scripts and agents. 

Each `ods find` row carries the `orgs.parquet` column names. Later releases may add joined or derived fields.

The JSON output of other commands is not yet tested for stability across releases. If you need this, open an issue.

### Notices

`ods find` and `ods cite` work from local files. When the release they read is
the newest in the workspace and more than 45 days old, they end with one line on
stderr:

```text
* 2026-08-28 release is 52 days old. Run `ods pull` to check for a newer one.
```

A release pinned older on purpose with `ods use` gets no notice, and neither do
machine formats such as `--format json`.

### Table width

A `find` or `role` table's width, in order: `COLUMNS` if set to a positive number;
the terminal's width when stdout is a terminal; otherwise no wrapping at all, so
each row stays on one line and a table written to a file or pipe doesn't depend on
the terminal it happened to run in. `*` notice lines follow the same rule: wrapped
to `COLUMNS` or the terminal, and left whole otherwise.

### Reproducibility guarantee

`ods make` builds byte-identical Parquet files from the same TRUD zip, on any
machine. This is guarantee D1 in [tests.md](./tests.md#the-data), and a test
fails if it stops being true.
