# ods find

Search NHS organisations and sites across names, codes, locations, and roles.

`ods find` searches the local Parquet dataset (`orgs.parquet`) and renders tabular results or exports to TSV, CSV, and JSON. JSON and CSV rows use the `orgs.parquet` column names today, but that shape [is not versioned yet](./cli.md#json-output-is-not-yet-stable).

## Usage

```text
ods find [QUERY] [OPTIONS]
```

### Options

| Option | Description |
| :--- | :--- |
| `[QUERY]` | Positional query matching organisation **name only** with relevance ranking |
| `--code <CODES>` | Filter by exact ODS code (repeatable and comma-separated, e.g. `--code A82608,RJZ`) |
| `--in <PLACE>` | Filter by exact town, county or country, or by postcode district (repeatable and comma-separated) |
| `-r, --role <ROLES>` | Filter by role codes (e.g. `RO76`) or curated names (repeatable and comma-separated) |
| `--gp` | Shortcut for GP practices (`RO76,RO227,RO315`) |
| `--dentist` | Shortcut for dental practices (`RO110,RO65`) |
| `-a, --all` | Include closed and inactive organisations; `find` shows only open ones by default (adds a Status column) |
| `-v, --verbose` | Show full role set in stored order without `+N` de-emphasis |
| `-s, --sort <SORT>` | Explicit sort order: `code`, `name`, or `postcode` (overrides default relevance ranking) |
| `-f, --format <FORMAT>` | Output format: `table` (default), `markdown`, `csv`, `json`, or `tsv` |
| `-i, --input <DIR>` | Directory containing Parquet files (defaults to active release) |
| `--plain` | Disable ANSI coloured output |
| `--sql` | Print the DuckDB query for these filters instead of running them |
| `-h, --help` | Print help |

## Search model and ranking

### Name matching and ranking
The positional argument matches the organisation `name` column. Matching ignores spacing,
hyphens and punctuation, so `st marys` finds `ST MARY'S` and `healthcare` finds `HEALTH CARE`.

Without `--sort`, rows whose name contains the query exactly as typed — before that
normalising strips it — sort first; the rest, matched only once punctuation and case are
stripped, follow. Within each group, rows sort by `ods_code`.

`--sort <field>` (e.g. `--sort name`) replaces this ranking with an explicit sort.

### Location filtering (`--in <place>`)
`--in` checks every value against four columns at once: `town`, `county`, `country` and `postcode`.

- **Places match whole.** Town, county and country must equal the value, ignoring case, apostrophes and punctuation. `--in "kings lynn"` finds KING'S LYNN, and `--in london` does not find LONDONDERRY.
- **Postcodes match by district, sub-district or whole postcode.** A value ending in a digit is a district and includes its lettered sub-districts: `--in N1` returns N1 and N1C, `--in SW1` returns SW1A to SW1Y, and `--in LA1` doesn't return LA10. A value ending in a letter is a sub-district and matches only itself (`--in N1C`). A whole postcode matches with or without its space (`LA10 5DL`, `LA105DL`). When a district includes sub-districts, a `*` line names them. Sector values (`"N1 1"`) match nothing and suggest the districts that exist. Two-character districts such as `N1` are accepted; any other value needs at least three characters.
- **Values add, flags narrow.** `--in durham --in cumbria` and `--in durham,cumbria` return rows in either place. Adding `--gp` or `--role` keeps only rows that match both. Commas always separate values, so the handful of towns recorded with a comma in their name (`ENFIELD, LONDON`) can't be matched.
- **Every matched field is named.** In `table` and `markdown`, a `Matched` column lists each field the row matched, in schema order: `town`, `county`, `postcode`, `country`. Machine formats carry no match field.
- **A place that matches nothing says so.** A value that matches no row in the file searched gets `! No location matches '<value>'` on stderr, with up to three suggestions, and the rows the other values matched are still returned. When no value matches anything, `ods find` prints `✖ No location matches '<value>'` and exits 1.
- **Longer place names are offered.** When `--in` returns rows, `* Also:` names up to three other places whose names start with the value and a space, most rows first: `--in newcastle` offers `"NEWCASTLE UPON TYNE" (1561)`.

`--in` returns what each record's address says, and doesn't correct it. Town is the post town and county is the postal county, so neither follows council or NHS boundaries:

- **`--in liverpool`** returns records whose post town is LIVERPOOL. Bootle and Prescot have L postcodes but post towns of their own, so ask for them by name (`--in liverpool,bootle,prescot`) or by district (`--in L20`).
- **`--in london`** returns records whose post town is LONDON. Outer boroughs have post towns of their own, such as HARROW and CROYDON.
- **County is recorded unevenly.** Records with post town HARROW give MIDDLESEX, GREATER LONDON or no county at all, so `--in "greater london"` finds only the records that say so.
- **Malformed postcodes stay as recorded.** 250 inactive records have postcodes that aren't in the standard full shape, such as `HR1 3` or `WA16 OED` (a letter O for a zero), all of them closed by 2017-03-31. A whole-postcode search doesn't find them; `--in HR1` and `--in WA16` do.

### Role filtering (`--role <roles>`)
`--role` accepts both `RO\d+` codes and curated role names. Multiple roles repeat with OR semantics. Unknown role names fail with suggestions and code shortcuts:

```text
✖ No role named 'General Practice'
  Did you mean: GP Practice · Scottish GP Practice · Northern Ireland GP Practice
  Or use codes: ods find --role RO76,RO227,RO315
```

Discover role codes and holder counts using `ods role [QUERY]`. Holders are open organisations; `ods role --all` counts closed and inactive ones too.

### Role shortcuts (`--gp`, `--dentist`)
Convenience flags expand directly into `--role` with OR semantics:
- `--gp`: Expands to England, Scotland, and Northern Ireland GP registers (`RO76,RO227,RO315`).
- `--dentist`: Expands to general and private dental practices (`RO110,RO65`).

### Role set display and de-emphasis
In tabular output, `find` displays descriptive roles first and tucks low-signal container/regulatory roles (`Prescribing Cost Centre`, `Social Care Site`, `Registered under Care Standards Act 2000`, `ePACT System`, `Foundation Trust`) behind a `+N` count:

```text
| ODS Code   | Name                                          | Postcode  | Roles                     | Class
| A82608     | SEDBERGH MEDICAL PRACTICE                     | LA10 5DL  | GP Practice +1            | org
| RJZ        | KING'S COLLEGE HOSPITAL NHS FOUNDATION TRUST  | SE5 9RS   | NHS Trust, Hospice +1     | org
```

Pass `-v, --verbose` to view the full role set in stored order. `--format json` and `--format csv` always export the complete `role_codes` and `role_names` lists in stored order.

### Normalisation
`ods` uppercases what you type. Stored values are already in capitals and are compared as written. Matching strips apostrophes, replaces symbols with spaces and collapses whitespace. Tables, JSON and CSV show each value as the source wrote it.

### Pipe workflows (`--format tsv`)
`--format tsv` outputs the same five columns as the table (six with `--all`) separated by tabs, with **no header row** and no borders. This makes it pipe directly into fuzzy finders and standard Unix tools (`fzf`, `sk`, `cut`, `awk`, `xargs`):

```bash
# Interactive search with fzf
ods find --format tsv | fzf

# Pick an organisation and inspect its full profile
ods find --format tsv | fzf | cut -f1 | xargs ods info
```

