# Parquet schemas

- [`orgs.parquet`](#orgsparquet)
- [`roles.parquet`](#rolesparquet)
- [`relationships.parquet`](#relationshipsparquet)
- [`successions.parquet`](#successionsparquet)

`ods make` compiles the NHS Organisation Data Service release into four Parquet
files. Point `duckdb` at them and go — no database, no server, no API key.

> _834MB of NHS XML, become 28MB and four tables you can query from a laptop._

```console
$ duckdb -c "SELECT ods_code, name, role_names
             FROM 'ods_data/current/orgs.parquet'
             WHERE status = 'active' AND town = 'SEDBERGH'"
┌───────────┬──────────────────────────────────────────┬────────────────────────────────────────┐
│ ods_code  │                   name                   │               role_names               │
│  varchar  │                 varchar                  │               varchar[]                │
├───────────┼──────────────────────────────────────────┼────────────────────────────────────────┤
│ 8GJ58     │ PARKER M JUNE (ACUPUNCURIST)             │ [Non-NHS Organisation]                 │
│ A82608    │ SEDBERGH MEDICAL PRACTICE                │ [Prescribing Cost Centre, GP Practice] │
│ A82608001 │ DR LUMB W & PARTNER                      │ [Branch Surgery]                       │
...
```

Every query needs that `status = 'active'` unless it means to include closed
organisations: `orgs.parquet` holds all of them, and `status` is the filter.

The source is one large XML document which is awkward to query. These tables are that document, rearranged.
NHS England documents the model behind it in the [ODS data model reference][ods-model].

The parquet files are ZSTD level 3, pinned so that rebuilding a release gives
byte-identical output. Paired with a declared metadata subset that excludes tool
and builder identity, anyone rebuilding from the same TRUD zip gets bit-identical
Parquet tables.

Adopt these schemas freely. If you publish ODS data, or anything shaped like it, use
them as they are — every tool that shares a schema can read every file written to it.

Worked examples live in [queries.md]. Note that the ODS records every entity that works with the NHS;
schools and care homes outnumber GP Practice rows. Other surprises in the data are listed in the
[Know this](#know-this) section.

## The tables

| File | Rows | What's in it |
| :--- | ---: | :--- |
| `orgs.parquet` | 370,917 | Every organisation and site ODS publishes: 217,059 `active` and 153,858 `inactive`. **Start here, and filter on `status`.** |
| `roles.parquet` | 529,348 | Row for each role an org holds |
| `relationships.parquet` | 769,527 | Row per relationship between two orgs |
| `successions.parquet` | 24,373 | Row per transition, split, or merger of duties between orgs |

Every table holds everything ODS publishes, closed organisations included, and carries
a status column so you choose what to ask: `orgs.status`, `roles.role_status` and
`relationships.rel_status`. `orgs` is sorted with active rows first, so
`WHERE status = 'active'` reads only the leading row groups, even over HTTP.

The tables cover every organisation in both files of a TRUD release: `fullfile.zip`,
NHS England's live product, and `archive.zip`, which holds what it has moved out once
an organisation closed before the published cut-off (currently 31 March 2017).
[docs/nhs.md](./nhs.md#the-archive-product) explains the split, and `ods make` merges it away.
That is why `orgs` matches the manifests' `RecordCount` totals (392,906) less the
21,989 stubs that repeat a record the other file holds in full.

That's not the same as every organisation ODS has ever known: closed entities seem to be kept indefinitely,
but some get deleted without notice. 186 codes in the May 2026 release are simply absent from August.
So the most complete history is the union of all releases, [not the newest file](./queries.md#deletions-happen-so-keep-your-snapshots).

This is where the parquet and duckdb (or pandas, polars, pyarrows etc) get interesting, as it lets us [query over all releases](./queries.md#across-releases) at once.

```sql
-- find total and active entities per dataset release
SELECT trud_release_date, count(*) AS n_rows,
       count(*) FILTER (WHERE status = 'active') AS n_active
FROM read_parquet('ods_data/releases/*/orgs.parquet')
GROUP BY 1 ORDER BY 1;
```
```
┌───────────────────┬────────┬──────────┐
│ trud_release_date │ n_rows │ n_active │
│       date        │ int64  │  int64   │
├───────────────────┼────────┼──────────┤
│ 2026-05-29        │ 368495 │   216564 │
│ 2026-06-26        │ 369400 │   216417 │
│ 2026-07-31        │ 370257 │   216886 │
│ 2026-08-28        │ 370917 │   217059 │
└───────────────────┴────────┴──────────┘
```


## `orgs.parquet`

One row per organisation or site, active or not, sorted by `status` (`active` first) then `ods_code`.
`ods_code` is unique. Where the source published two complete records for one code, `ods`
keeps the full file's over the archive's, and within one file the one whose operational
period starts later; [nhs.md](./nhs.md#a-code-published-twice) names the two cases.

| Column | Type | Null | Description |
| :--- | :--- | :--- | :--- |
| `ods_code` | `VARCHAR` | no | The ODS code. Join on this. |
| `name` | `VARCHAR` | no | Verbatim from ODS, which publishes in upper case |
| `status` | `VARCHAR` | no | `active` or `inactive`. Filter on it: `WHERE status = 'active'` is what's open today |
| `record_class` | `VARCHAR` | no | `org` or `site` |
| `role_codes` | `VARCHAR[]` | no | Sorted and deduplicated. An active organisation lists its active roles; an inactive one lists every role it held |
| `role_names` | `VARCHAR[]` | no | Curated names, aligned position-for-position with `role_codes` |
| `primary_role_code` | `VARCHAR` | no | The role ODS designates primary |
| `address` | `VARCHAR` | yes | Address lines 1–3, joined with `, ` |
| `town` | `VARCHAR` | yes | |
| `county` | `VARCHAR` | yes | |
| `postcode` | `VARCHAR` | yes | Always space-separated, so `LIKE 'SW9 %'` is safe |
| `country` | `VARCHAR` | yes | England, Wales, Scotland, Northern Ireland, Isle of Man or Channel Islands |
| `uprn` | `VARCHAR` | yes | Unique Property Reference Number |
| `telephone` | `VARCHAR` | yes | |
| `website` | `VARCHAR` | yes | Lower-cased; the source is upper case |
| `predecessor_codes` | `VARCHAR[]` | no | Every ancestor, not just the previous one |
| `successor_codes` | `VARCHAR[]` | no | Every descendant. The exact inverse of `predecessor_codes` |
| `legal_start` | `DATE` | yes | Statutory dates. Populated on 19% of rows |
| `legal_end` | `DATE` | yes | |
| `operational_start` | `DATE` | yes | Populated on every row |
| `operational_end` | `DATE` | yes | Set on inactive rows, and on 623 active ones with a closure already scheduled |
| `last_changed` | `DATE` | yes | ODS's own last-modified date |
| `trud_release_date` | `DATE` | no | The release this row came from |

`role_names` is the column you want for _what is this thing_. `primary_role_code`
is often administrative rather than descriptive — it files most GP practices under
`RO177 Prescribing Cost Centre`.

`record_class` uses readable values `org` and `site` instead of ODS's internal `RC1`/`RC2` codes. They are not join keys, and the readable form is the useful one.

## `roles.parquet`

One row per role held. An organisation holding three roles has three rows.

| Column | Type | Null | Description |
| :--- | :--- | :--- | :--- |
| `ods_code` | `VARCHAR` | no | The organisation holding the role |
| `role_code` | `VARCHAR` | no | ODS role code, e.g. `RO76` |
| `role_name` | `VARCHAR` | no | Our curated name, e.g. `GP Practice` |
| `is_primary` | `BOOLEAN` | no | Whether ODS designates this the primary role |
| `role_status` | `VARCHAR` | no | `active` or `inactive` |
| `legal_start` | `DATE` | yes | |
| `legal_end` | `DATE` | yes | |
| `operational_start` | `DATE` | yes | |
| `operational_end` | `DATE` | yes | |
| `role_id` | `VARCHAR` | no | ODS's identifier for this holding. Stable across releases |
| `trud_release_date` | `DATE` | no | |

The full vocabulary is `SELECT DISTINCT role_code, role_name FROM 'roles.parquet'`
— 209 of them in the current release. `ods role` lists them with holder counts.

Reach for this table when you want a role's _history_. For _what an organisation
is now_, `orgs.role_codes` already has it, deduplicated, without a join: for an active
organisation it holds only the roles that are still active, so `Y07792`, which also holds
an ended `RO72`, lists `[RO177, RO321]`, while an inactive organisation such as `G7811339`
keeps every role it held (`[RO59, RO88]`, the second of them ended too).

## `relationships.parquet`

One row per relationship. Relationships are one-way and live on the source
record, so `source_code` is the organisation making the statement.

| Column | Type | Null | Description |
| :--- | :--- | :--- | :--- |
| `source_code` | `VARCHAR` | no | The organisation the relationship belongs to |
| `target_code` | `VARCHAR` | no | The organisation it points at |
| `rel_code` | `VARCHAR` | no | ODS relationship code, e.g. `RE4` |
| `rel_name` | `VARCHAR` | no | ODS's own wording, e.g. `IS COMMISSIONED BY` |
| `rel_status` | `VARCHAR` | no | `active` or `inactive` |
| `legal_start` | `DATE` | yes | |
| `legal_end` | `DATE` | yes | |
| `operational_start` | `DATE` | yes | |
| `operational_end` | `DATE` | yes | |
| `rel_id` | `VARCHAR` | no | ODS's identifier for this relationship. Stable across releases |
| `trud_release_date` | `DATE` | no | |

The types in the current release:

| Code | Name | Rows | Means |
| :--- | :--- | ---: | :--- |
| `RE2` | `IS A SUB-DIVISION OF` | 237 | A department or programme of the target |
| `RE3` | `IS DIRECTED BY` | 6,849 | Directed through policy, legal authority or contract |
| `RE4` | `IS COMMISSIONED BY` | 161,670 | The target commissions services from the source |
| `RE5` | `IS LOCATED IN THE GEOGRAPHY OF` | 333,979 | Physically inside the target's boundary — **being retired**, see below |
| `RE6` | `IS OPERATED BY` | 253,800 | The target runs and manages the source |
| `RE8` | `IS PARTNER TO` | 8,346 | The source is part of the target partnership or network |
| `RE9` | `IS NOMINATED PAYEE FOR` | 1,698 | Payee for the target Primary Care Network |
| `RE10` | `IS COVID NOMINATED PAYEE FOR` | 913 | All closed; kept for history |
| `RE11` | `IS CONSTITUENT OF` | 2,035 | Cost centre to Sub-ICB reporting entity. London ICBs only |

`rel_name` is ODS's wording, carried through unchanged, and the meanings above are
theirs too — from the [ODS data model's relationships reference][ods-relationships].

## `successions.parquet`

One row per succession event.

| Column | Type | Null | Description |
| :--- | :--- | :--- | :--- |
| `predecessor_code` | `VARCHAR` | no | The organisation that was succeeded |
| `successor_code` | `VARCHAR` | no | The organisation that succeeded it |
| `legal_start` | `DATE` | no | When it took effect |
| `succession_id` | `VARCHAR` | no | ODS's identifier for this event. Stable across releases |
| `trud_release_date` | `DATE` | no | |

This is the edge list. `orgs.successor_codes` and `orgs.predecessor_codes` hold
the whole chain — not just the next step — so you never have to walk it yourself.

## Joining the tables

Every supporting table carries ODS codes that point back to `orgs`.

| Table | Join to `orgs` on | One row per |
| :--- | :--- | :--- |
| `roles.parquet` | `ods_code` | role an organisation holds or has held |
| `relationships.parquet` | `source_code` for an organisation's own relationships, `target_code` for those pointing at it | relationship |
| `successions.parquet` | `predecessor_code` for what an organisation became, `successor_code` for what it replaced | succession |

**Add `WHERE o.status = 'active'` only when you mean active organisations.** Supporting
tables keep rows for closed organisations, and `orgs` holds those too. All 529,348 `roles`
rows find their organisation in `orgs`; 314,195 find an active one.

`roles` names its key the same way `orgs` does, so `USING` reads well:

```sql
SELECT count(*)
FROM 'ods_data/current/roles.parquet' r
JOIN 'ods_data/current/orgs.parquet' o USING (ods_code);
-- 529348
```

**Relationships and successions have a direction, and the column name says which side
you are on.** A relationship belongs to the organisation making the statement,
`source_code`, and points at `target_code`. Joining on `source_code` alone finds an
organisation's own relationships and none that point at it: the ICB `01K` states 5
relationships and is the target of 674, and 44,877 organisations only ever appear as a
target. Join each side under its own alias:

```sql
SELECT s.name AS organisation, rel.rel_name, t.name AS target
FROM 'ods_data/current/relationships.parquet' rel
JOIN 'ods_data/current/orgs.parquet' s ON rel.source_code = s.ods_code
JOIN 'ods_data/current/orgs.parquet' t ON rel.target_code = t.ods_code
WHERE rel.source_code = 'A82608' AND rel.rel_status = 'active'
ORDER BY rel.rel_code;
-- SEDBERGH MEDICAL PRACTICE · IS COMMISSIONED BY · NHS LANCASHIRE AND SOUTH CUMBRIA ICB - 01K
-- SEDBERGH MEDICAL PRACTICE · IS OPERATED BY     · NHS LANCASHIRE AND SOUTH CUMBRIA ICB - 01K
-- SEDBERGH MEDICAL PRACTICE · IS PARTNER TO      · WESTERN DALES PCN
```

Most succession questions need no join. `orgs.successor_codes` and
`orgs.predecessor_codes` already hold the whole chain; reach for
`successions.parquet` when you need the date each step took effect.

**Never `NATURAL JOIN` these tables.** `roles` shares more than its key with `orgs`:
`legal_start`, `legal_end`, `operational_start`, `operational_end` and
`trud_release_date` too, so a natural join matches on the dates as well and silently
drops rows.

```sql
SELECT count(*)
FROM 'ods_data/current/roles.parquet' NATURAL JOIN 'ods_data/current/orgs.parquet';
-- 78680, not 529348
```

Use `USING (ods_code)` or an explicit `ON`.

## How the schema works

Six rules, and they explain nearly every decision here.

- **Every table holds everything, and a status column says what's open.** `orgs.status`,
  `roles.role_status` and `relationships.rel_status` are the filter; no table is an
  active-only subset of another, and `ods find` and `ods role` apply it for you unless
  you pass `--all`.
- **Codes are ODS's, names are ours.** `role_code` is assigned by ODS and never
  changes. `role_name` is our rendering of it, because the source is upper case
  and sometimes abbreviated (`REG'D UNDER PART 2 CARE STDS ACT 2000`) or
  misspelled (`MANAGMENT`). Save codes in your queries; the names are for reading.
- **Codes come before names**, so the join key is the first thing you see.
- **Lists are empty, never NULL.** An organisation with no successors has `[]`.
  Anything that can legitimately hold more than one value is a list, so we never
  pick one on your behalf.
- **Every table reads the same way** — what it is, where it is, whether it's
  live and when, then identifiers. Once you know one table you can skim the rest.
- **We publish what ODS states.** Where answering a question takes a judgement
  call — *which ICB is relevant to this care home?* — that judgement lives in
  `ods`, where it can be revised, not baked into files people have cited. Use
  `ods find`, `ods info` and the [recipes in queries.md][queries.md].

## Point-in-time queries

`orgs.parquet` holds every organisation as of the release date, with its status then.
For an earlier date, filter on the operational dates:

```sql
SELECT ods_code, name
FROM 'orgs.parquet'
WHERE operational_start <= DATE '2019-03-31'
  AND (operational_end IS NULL OR operational_end > DATE '2019-03-31');
```

That gives you _today's record of 2019_, which isn't the same as _the 2019
release_. ODS backfills end dates as information arrives, and entities are
registered, closed, reopened and occasionally deleted between releases. If you
need what ODS knew at the time, keep the release from the time — it's
[measured here](./queries.md#which-release-should-you-use).

Keep the releases and that becomes the interesting part: closures, renames and
reparenting are all recoverable [across an archive](./queries.md#across-releases).

## Data Package

`ods make` and `ods pull` write a `datapackage.json` beside the Parquet files to surface the
provenance, source license, and data schema in an open [Data Package v2][datapackage] format.

It's generated from metadata baked into the parquet files, to reduce the surface for determinism
issue around canonical JSON formatting. Instead it's a view we generate during `ods pull`

The four list columns in `orgs` are declared as Table Schema's `array` type.
Its `list` type, with `itemType: string`, is the closer fit for a Parquet `LIST<VARCHAR>`, but
it's a delimited string in the spec's lexical form, and the published v2 profile
(`datapackage.org/profiles/2.0`) has no `list` type, so a descriptor that used it would fail that
profile. The fix is merged upstream, to be published as v2.1 ([datapackage#1089][dp-1089]). When
a v2.1 profile is published, the view declares the four columns as `list` with `itemType:
string`. The view isn't part of any digest, so that needs no dataset version bump.

Note that at the time of writing frictionless-py reads Parquet through pandas, which hands list cells over as NumPy arrays, so every list column reads as a type error. It also skips counting and hashing Parquet files, so every declared `bytes` and `hash` fails. Both are bugs in its Parquet reader, not in the release. It needs `pandas` installed alongside the `parquet` extra ([#1773][fl-1773]), and loads each file whole ([#1203][fl-1203]).

To check a release's files against its descriptor today:

```console
$ ods make datapackage -o - | jq -r '.resources[] | "\(.hash[7:])  \(.path)"' | shasum -a 256 -c
orgs.parquet: OK
roles.parquet: OK
relationships.parquet: OK
successions.parquet: OK
```

## Provenance

Every Parquet file carries one key-value metadata key, `datapackage`, holding the release's
provenance as compact JSON in Data Package vocabulary: the dataset's name and version, its licence
and attribution, who holds the rights and distributes, and the TRUD release it was built from,
with that release's SHA-256 and size. All four files carry the same object, and nothing else is
added to the key-value metadata (Arrow writes its own `ARROW:schema` beside it, as it always has).
Read it with DuckDB:

```sql
SELECT decode(key), decode(value) FROM parquet_kv_metadata('orgs.parquet');
```

The object holds nothing about who built the release or how the archive was checked. The `ods`
that built a published dataset is recorded in its release index row (`tool_version` and
`tool_git_sha`) and, as signed proof, in the CI attestation for the same manifest digest. `ods
cite` renders it as a citation. [provenance.md](./provenance.md) has every field, and whose word
each is.

The dataset version is part of the object, inside `version`, so every file says which cut it is.
That's a deliberate cost: relabelling a release with a new dataset version (the 1.0.0 republish,
say) changes every Parquet file's bytes, even though the rows are the same, and the release is
downloaded again.

To verify a release: rebuild from the TRUD archive, using any version of `ods` that builds the
same dataset version, and compare. The Parquet files match byte for byte, and so do the manifests
`ods` rebuilds from them, and their digests: the object holds only facts about the archive and
the dataset version, so it doesn't differ between builders.

## Versioning

Three numbers, three jobs:
- `trud_release_date` — which source? (every Parquet row, the embedded object's `sources[0].version`, and the manifest's `fyi.ods.source.version` annotation).
- `dataset_version` — which cut, and which attempt at it? (SemVer, after the `_` in the embedded object's `version` and the manifest's `org.opencontainers.image.version`, and on the release index row).
- `ods` crate version — which tool? (`Cargo.toml`; for a published dataset, the `tool_version` on its release index row).

`dataset_version` is global and monotonic. It identifies a *cut* — the state of the tool and rules at the moment of packing — so once it moves, every release packed afterwards carries the new number.

What each part of `dataset_version` promises, and what `ods` does with a release at each:

| Bump | Means | An `ods` built for 1.0.0 reading it |
| :--- | :--- | :--- |
| patch, e.g. 1.0.1 | same schema, rebuilt | reads it silently |
| minor, e.g. 1.1.0 | columns added, nothing removed or redefined | reads it, and suggests upgrading `ods`, which may not show the new columns |
| major, e.g. 2.0.0 | a breaking change: a column removed, renamed, or given a new meaning | tries anyway, prints the output, then warns that the results may be wrong, and exits 1 |

**0.x is for iterating.** A breaking change may land in any 0.x release, and `ods` reads
0.x releases without version warnings. **1.0.0 is a statement of intent** to support the
schema from then on. It is identical to the last 0.x, and every release date is republished
as 1.0.0. Every Parquet file carries its dataset version, so the republished files' bytes differ
from the 0.x ones and `ods pull` downloads them again: the rows are the same, the provenance each
file carries isn't.

- `ods pull` installs the newest release at the major version it reads, and says when a newer major exists.
- A workspace holds one version per release date, in `releases/<date>/`, so `releases/*/` queries count each release once. Pulling a new version of a date replaces it, reusing any file whose bytes haven't changed.
- Upgrading `ods` keeps older releases readable: a column added in a minor version is read as optional, and only the columns present at the start of the current major are required.

**A patch bump means "prefer this", not "the derivation changed."** After a one-off bad build is republished at 1.0.1, every subsequent month is byte-identically derived to the 1.0.0 months before it and still carries 1.0.1.


## Know this

There are surprises lurking in the data...

**`RE5` is being retired upstream.** It's 44% of `relationships.parquet` today,
and NHS ODS has marked it legacy: *"not part of the formal documentation or
specification for the FHIR R4 ODS API"*. They direct developers to postcode
geography data instead ([ODS relationships reference][ods-relationships]).

_TODO: JOIN from `orgs.postcode` to geography. The magic of parquet is can link to
other databases rather than make this one capture everything._

**Filter on `status`, not on `operational_end IS NULL`.** 603 active
organisations already have a closure date scheduled. Treating a null end date as
"open" silently drops them — [counted here](./queries.md#traps).

**`1900-01-01` is filler, not a date.** It appears as `operational_start` on
12,470 rows, mostly schools. Anything taking a `min()` or measuring an age needs
to handle it. [Documented here](./queries.md#other-things-to-check-before-you-trust-a-result).

**Names aren't identifiers.** 1,680 organisations are called `DENTAL SURGERY`.
Join on `ods_code`, always — [worked through here](./queries.md#names-are-not-identifiers).

[queries.md]: ./queries.md
[datapackage]: https://datapackage.org/
[fl-1773]: https://github.com/frictionlessdata/frictionless-py/issues/1773
[fl-1203]: https://github.com/frictionlessdata/frictionless-py/issues/1203
[dp-1089]: https://github.com/frictionlessdata/datapackage/issues/1089
[Croissant]: https://docs.mlcommons.org/croissant/
[ods-model]: https://www.odsdatasearchandexport.nhs.uk/referenceDataCatalogue/ODS-Data-Model_571324843.html
[ods-relationships]: https://www.odsdatasearchandexport.nhs.uk/referenceDataCatalogue/Relationships_571324965.html
