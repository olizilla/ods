# Tests

These are the behaviours `ods` guarantees. The test suite exists to protect
them: each test enforces one, and a failing test names the one that broke.

## The data

- **D1. Same zip, same bytes.** `ods make` builds byte-identical Parquet files from the same TRUD zip at the same dataset version, on any machine, operating system, CPU architecture, Rust toolchain or working directory.
- **D2. The schema is the contract.** The Parquet files match `data/datapackage.json` and [parquet.md](./parquet.md): the same tables, columns, column order and types.
- **D3. Facts land where the XML puts them.** Each date, status and code is read from the element it belongs to. A role's status stays on the role, and legal and operational dates stay distinct.
- **D4. `orgs_all` is the whole release, `orgs` its active part.** `orgs_all` holds every record in the release XML, matching the manifest's `RecordCount`, and `orgs` holds the active ones.

## Releases

- **R1. Unverified files are clearly flagged.** `ods pull` gives a file its name only after its SHA-256 matches the release manifest. A file that fails on every mirror lands as `<name>.bad-sha`, so the verified files stay usable. `current` stays where it was, and the pull names each bad file with both digests and exits 1.
- **R2. The index's digest decides.** A fetched or supplied index that contradicts a digest in the index built into `ods` stops the pull. A registry tag that names a different digest from the index is reported, and the index's release is what gets installed.
- **R3. ods.fyi first, then ghcr.io.** `ods` asks ods.fyi first for the index and every file, and falls back to ghcr.io. It finds releases only through the index, and `ods pull` alone installs the newest release there.
- **R4. Pulls repair, and repeat for free.** Pulling a verified release a second time downloads nothing. A local file that no longer matches its digest is fetched again.
- **R5. Withdrawn releases are named, loudly.** `ods pull <date>` and `ods cite` still deliver a withdrawn release, then print the reason it was withdrawn and exit 1. `ods pull` with no date fetches the newest release that isn't withdrawn, and names any it skipped. A withdrawal published in a newer index applies to a release built into `ods`.
- **R6. `ods use` pins what you name.** It pins any release in the workspace, and warns when that release doesn't match the published one or can't be checked.
- **R7. A citation identifies one release.** Every `ods cite` format carries the dataset version and the manifest digest. A release whose source archive is unverified is cited with a warning.
- **R8. A dataset version says what an `ods` can read.** Before 1.0.0, `ods` reads every release without version warnings. From 1.0.0, a patch is the same schema. A minor adds columns: an `ods` with the same major reads it, and nudges to upgrade. A major is breaking: reading it prints the output, then a `✖` block, and exits 1. `ods pull` installs the newest release at its own major, names newer majors, and reuses local files whose digests already match. Local releases, one per date, stay readable after `ods` is upgraded.

## Building from source

- **B1. The source archive is checked.** An archive downloaded by `ods trud pull` is checked against TRUD's SHA-256. An archive supplied from disk is checked against the ods release index, then against the TRUD API when `TRUD_API_KEY` is set. A mismatch fails, and an archive neither source knows is recorded as unverified.
- **B2. The API key stays secret.** `TRUD_API_KEY` appears in no output and in no file `ods` writes.
- **B3. Provenance is carried, not invented.** `ods make` carries the archive's provenance into the release. An archive without a TRUD checksum builds with a warning, a directory without provenance fails, and a value nobody supplied reads `unknown`.
- **B4. Audit catches disagreement.** `ods audit` fails when a release's Parquet files, provenance or file list disagree with its source XML.
- **B5. Same files, same manifest.** `ods make oci` packs the same manifest digest from the same files in any directory, and refuses a layout whose digests disagree.
- **B6. Releases come from a clean tree.** `ods make release` refuses a dirty working tree, a tool commit that differs from the build's, a duplicate index row, and unverified provenance.
- **B7. `ods make` leaves the active release alone.** Building a release does not change which release is active.

## Querying

- **Q1. Names match however they are punctuated.** `ods find` ignores spacing, hyphens and punctuation in names, and ranks exact matches first.
- **Q2. `--in` matches whole places.** A value matches a row whose `town`, `county` or `country` equals it, ignoring case, apostrophes and punctuation. `--in "King's Lynn"` and `--in "kings lynn"` return the same rows, as do `--in stoke-on-trent` and `--in "stoke on trent"`. Part of a name matches nothing: `--in london` does not return LONDONDERRY.
- **Q3. Postcodes match by district, then sector.** `--in LA1` returns district LA1 and not LA10, while `--in SW1` returns SW1A to SW1Y, since a district also covers its lettered sub-districts. `--in "LA1 5"` returns sector LA1 5, and a full postcode returns that postcode with or without its space. A two-character district such as `--in N1` is accepted; any other value needs at least three characters.
- **Q4. Values add rows, flags remove them.** Values given to `--in`, repeated or comma-separated, combine as a union. Different flags combine as an intersection, so adding `--gp` or `--role` never adds rows. A comma always separates values, so a town recorded with a comma in it, such as `ENFIELD, LONDON`, cannot be matched.
- **Q5. Every matched field is named.** With `--in`, the `table` and `markdown` formats gain a `Matched` column listing each field the row matched, in schema order: `town`, `county`, `postcode`, `country`. Machine formats carry no match field.
- **Q6. A place that matches nothing says so.** A value that matches no row in the file searched gets `! No location matches '<value>'` and up to three suggestions, and the rows the other values matched are still returned. When no value matches anything, it is an error, `✖ No location matches '<value>'`, and `ods find` exits 1.
- **Q7. Near misses are offered.** When `--in` returns rows, `* Also:` names up to three other places that begin with the value followed by a space, most rows first: `--in newcastle` offers `"newcastle upon tyne" (1561)`.
- **Q8. `--sql` returns what `find` returned.** Run in DuckDB, the query returns the same set of rows as `ods find`. The order matches wherever we can make it match, and the tests check it, but the order is not guaranteed.
- **Q9. Rows are `orgs` rows.** In `--format json` and `csv`, `ods find` rows use the `orgs` column names.
- **Q10. `info` shows the whole record.** `ods info <code>` accepts a code in any letter case, shows its relationships and succession chain, and fails on an unknown code.
- **Q11. Holders are active holders.** `ods role` counts the active organisations that actively hold each role.

## Output

- **O1. stdout is what you asked for.** In the human formats (`table`, `markdown`, and `ods cite`'s `text`), that is the whole reading: the `* Source:` header, including its `!` line when you run from inside a different release, notices such as `* Also:`, the table and its footer, or the citation. Errors, warnings and progress go to stderr in every format, and nudges go to stderr with the human formats only. `ods pull`, `ods trud pull`, `ods make` and `ods use` report their work on stderr.
- **O2. Machine formats are only data.** With `json`, `csv`, `tsv`, `ndjson`, `bibtex`, `csljson` or `apa`, stdout carries the records and nothing else. Colour codes appear only when stdout is a terminal.
- **O3. Every query names its release.** In the human formats, `ods find`, `ods info`, `ods role` and `ods cite` open with a `* Source:` line naming the release they read.
- **O4. Reading writes nothing.** `ods find`, `ods info`, `ods role` and `ods cite` create and change no files. Without a workspace, they fail with a message that says what to run.
- **O5. Output changes on purpose.** Each command's human-readable output is compared against one committed snapshot, which is updated whenever that output is meant to change.
- **O6. Only `ods pull` and `ods trud` use the network.** Every other command works from local files. When the release being read is the newest in the workspace and more than 45 days old, `ods find` and `ods cite` print one line: ``* 2026-08-28 release is 52 days old. Run `ods pull` to check for a newer one.``

## The workspace

- **W1. An explicit release directory wins.** A command given a release directory reads that release, whichever release is active.
- **W2. A workspace is marked by `_releases.json`.** A directory is a workspace when it holds a `_releases.json` that parses as a release index with `_type` set to `ods_release_index`. That file holds the last blessed index: the one built into `ods`, or the bytes `ods pull` last fetched from ods.fyi or GitHub, saved exactly as served. An index passed with `--index` is used for that run only.
- **W3. The workspace is found by walking up.** Without an explicit path, `ods` checks the current directory and each parent, for a marked directory or a marked `ods_data` inside it. It stops at the first directory containing `.git`, at `$HOME`, or at the filesystem root.

## ods.fyi

- **H1. Named paths serve the release's own bytes.** `/orgs.parquet`, `/latest/…` and `/<date>/<version>/…` return the blobs of the manifest they name, and byte ranges of them.
- **H2. It is an OCI registry.** Any OCI client can pull a release from ods.fyi through the distribution API.
