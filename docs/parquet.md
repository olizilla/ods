# Parquet schemas

> _658MB of NHS XML, as five tables you can query from a laptop._

`ods make` compiles the NHS Organisation Data Service release into five Parquet
files. Point `duckdb` at them and go — no database, no server, no API key.

```console
$ duckdb -c "SELECT ods_code, name, role_names
             FROM 'ods_data/current/orgs.parquet'
             WHERE town = 'SEDBERGH'"
┌───────────┬──────────────────────────────────────────┬────────────────────────────────────────┐
│ ods_code  │                   name                   │               role_names               │
│  varchar  │                 varchar                  │               varchar[]                │
├───────────┼──────────────────────────────────────────┼────────────────────────────────────────┤
│ 8GJ58     │ PARKER M JUNE (ACUPUNCURIST)             │ [Non-NHS Organisation]                 │
│ A82608    │ SEDBERGH MEDICAL PRACTICE                │ [Prescribing Cost Centre, GP Practice] │
│ A82608001 │ DR LUMB W & PARTNER                      │ [Branch Surgery]                       │
│ D2E8H     │ AP SD THIRTEEN LIMITED                   │ [Pharmacy Headquarter]                 │
│ EE112233  │ SEDBERGH PRIMARY SCHOOL                  │ [School, Community School]             │
...
```

The source is one large XML document which is awkward to query. These tables are that document, rearranged.
NHS England documents the model behind it in the [ODS data model reference][ods-model].

The parquet files are ZSTD level 3, pinned so that rebuilding a release gives
byte-identical output. 

Worked examples live in [queries.md]; Note that the ODS records every entity that works with the NHS;
schools and care homes outnumber GP Practice rows. Other suprises in the data are listed in the
[Know This](#know-this) section.

## The tables

| File | Rows | What's in it |
| :--- | ---: | :--- |
| `orgs.parquet` | 216,886 | Every org and site active at the release date. **Start here.**  |
| `orgs_all.parquet` | 305,541 | Same columns, plus every closed or retired entity this release still carries |
| `roles.parquet` | 442,251 | Row for each role an org holds |
| `relationships.parquet` | 662,558 | Row per relationship between two orgs |
| `successions.parquet` | 11,568 | Row per transition, split, or merger of duties between orgs |

`orgs` is a subset of `orgs_all` that only includes active entities, to make it easier to ask
questions about what the NHS is today.

`orgs_all` is every record in that release's XML — it matches the manifest's `RecordCount` exactly.
That's not the same as every organisation ODS has ever known: closed entities seem to be kept indefinitely,
but some get deleted without notice. 165 codes in the May 2026 release are simply absent from July.
So the most complete history is the union of all releases, [not the newest file](./queries.md#the-archive-is-the-only-complete-list).

This is where the parquet and duckdb (or pandas, polars, pyarrows etc) get interesting, as it lets us [query over all releases](./queries.md#across-releases) at once.

```sql
-- find total and active entities per dataset release
SELECT trud_release_date, count(*) AS n_rows,
       count(*) FILTER (WHERE status = 'active') AS n_active
FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
GROUP BY 1 ORDER BY 1;

┌───────────────────┬────────┬──────────┐
│ trud_release_date │ n_rows │ n_active │
│       date        │ int64  │  int64   │
├───────────────────┼────────┼──────────┤
│ 2026-06-26        │ 304662 │   216417 │
│ 2026-07-31        │ 305541 │   216886 │
└───────────────────┴────────┴──────────┘
```


## `orgs.parquet` and `orgs_all.parquet`

Identical schemas. One row per organisation or site.

| Column | Type | Null | Description |
| :--- | :--- | :--- | :--- |
| `ods_code` | `VARCHAR` | no | The ODS code. Join on this. |
| `name` | `VARCHAR` | no | Verbatim from ODS, which publishes in upper case |
| `record_class` | `VARCHAR` | no | `org` or `site` |
| `role_codes` | `VARCHAR[]` | no | Every role held, sorted and deduplicated |
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
| `status` | `VARCHAR` | no | `active` or `inactive`. Always `active` in `orgs.parquet` |
| `legal_start` | `DATE` | yes | Statutory dates. Populated on 14% of rows |
| `legal_end` | `DATE` | yes | |
| `operational_start` | `DATE` | yes | Populated on every row |
| `operational_end` | `DATE` | yes | Set on inactive rows, and on 603 active ones with a closure already scheduled |
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
— 205 of them in the current release. `ods role` lists them with holder counts.

Reach for this table when you want a role's _history_. For _what an organisation
is now_, `orgs.role_codes` already has it, deduplicated, without a join.

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
| `RE2` | `IS A SUB-DIVISION OF` | 225 | A department or programme of the target |
| `RE3` | `IS DIRECTED BY` | 5,788 | Directed through policy, legal authority or contract |
| `RE4` | `IS COMMISSIONED BY` | 148,947 | The target commissions services from the source |
| `RE5` | `IS LOCATED IN THE GEOGRAPHY OF` | 292,107 | Physically inside the target's boundary — **being retired**, see below |
| `RE6` | `IS OPERATED BY` | 202,504 | The target runs and manages the source |
| `RE8` | `IS PARTNER TO` | 8,345 | The source is part of the target partnership or network |
| `RE9` | `IS NOMINATED PAYEE FOR` | 1,698 | Payee for the target Primary Care Network |
| `RE10` | `IS COVID NOMINATED PAYEE FOR` | 913 | All closed; kept for history |
| `RE11` | `IS CONSTITUENT OF` | 2,031 | Cost centre to Sub-ICB reporting entity. London ICBs only |

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

## How the schema works

Five rules, and they explain nearly every decision here.

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

`orgs.parquet` is active as of the release date. For an earlier date, filter
`orgs_all.parquet`:

```sql
SELECT ods_code, name
FROM 'orgs_all.parquet'
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

## Frictionless Data Package

`datapackage.json` describes each release in [Frictionless Table Schema][frictionless]
format. `roles`, `relationships` and `successions` validate with the Frictionless
framework. `orgs` and `orgs_all` use native Parquet list columns, which Table
Schema's flat-cell model doesn't cover — read those with DuckDB, Polars or Arrow,
which handle them natively.

## Provenance

Every release ships `_provenance.json` and `SHA256SUMS`, recording the TRUD
archive it came from and its verified SHA-256, the ODS publication date and
sequence number, and the commit that built it. That's enough to rebuild the
release and compare bytes. Hashes are uppercase throughout, matching TRUD.
`ods cite` renders it as a citation.

The same facts are in each file's Parquet key-value metadata, so a file separated
from its directory still knows where it came from.

## Know This

There are suprises lurking in the data...

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
to handle it. [Docuemtned here](./queries.md#other-things-to-check-before-you-trust-a-result).

**Names aren't identifiers.** 1,680 organisations are called `DENTAL SURGERY`.
Join on `ods_code`, always — [worked through here](./queries.md#names-are-not-identifiers).

[queries.md]: ./queries.md
[frictionless]: https://specs.frictionlessdata.io/table-schema/
[ods-model]: https://www.odsdatasearchandexport.nhs.uk/referenceDataCatalogue/ODS-Data-Model_571324843.html
[ods-relationships]: https://www.odsdatasearchandexport.nhs.uk/referenceDataCatalogue/Relationships_571324965.html
