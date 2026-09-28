<!-- Written by ods, which rewrites this file: edits won't last. -->
# `ods` workspace

NHS Organisation Data Service (ODS) data as Parquet files, pulled and verified by the `ods` CLI.
The data is NHS England's, under the Open Government Licence: `ods cite` gives the attribution
and how to cite the release you're reading.

## Find organisations

```console
ods find "royal free"
ods find --gp --in SW9
ods info RVJ
ods role RO76
```

## Keep it current

NHS England publishes a new release every few weeks. Run `ods pull` every few weeks to fetch the
newest; `ods find` and `ods cite` say when the release you're reading is getting old.

## From `ods` to SQL

`--sql` shows the query `ods find` runs, instead of running it:

```console
ods find --gp --in SW9 --sql
```

Pass it to DuckDB to run it there, or change it first:

```console
ods find --gp --in SW9 --sql | duckdb
```

Or query the files directly:

```sql
SELECT ods_code, name, postcode
FROM 'current/orgs.parquet'
WHERE status = 'active' AND town = 'SEDBERGH';
```

## What's here

- `current/`: the release `ods` reads, a link into `releases/`. Switch with `ods use <date>` or
  `ods use latest`.
- `releases/<date>/`: one directory per release, holding four Parquet tables and
  `datapackage.json`, which describes them.
- `_releases.json`: the cached release index.

Worked queries and the table reference: https://github.com/olizilla/ods/tree/main/docs
