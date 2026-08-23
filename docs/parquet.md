# Parquet schemas

The tables `ods make` produces from the NHS TRUD ODS XML release. Worked
examples, including how to query several releases at once, are in
[queries.md](./queries.md).

> **Pre-1.0.** This is the schema **as it will be at v1**. We publish v1 of the data once we're happy with this shape, and it's fixed from then on.

## The tables

| file | grain | rows (7.0.0) | purpose |
| :--- | :--- | ---: | :--- |
| `orgs.parquet` | one per active organisation or site | 216,886 | the main analytical table |
| `orgs_all.parquet` | one per organisation or site, active **and** inactive | 305,541 | historical work |
| `roles.parquet` | one per organisation per role | 442,251 | which roles an entity holds, and when |
| `relationships.parquet` | one per relationship | 662,558 | how entities relate |
| `successions.parquet` | one per succession | 11,568 | mergers, splits, renames |

`orgs` answers nearly everything about the NHS as it is today. The other three
are for deeper work and are designed to be joined against `orgs_all` rather than
read alone.

**Three tables of the same shape.** `roles`, `relationships` and `successions`
each hold one row per *thing that happened between an organisation and something
else*, each carrying its type inline rather than in a companion lookup table:

```
roles          one per organisation per role   role_code, role_name, dates, status
relationships  one per relationship            rel_type_code, rel_type_name, dates, status
successions    one per succession              dates
```

There is deliberately **no role vocabulary table**. `roles.parquet` carries
`role_name` inline, so the vocabulary is one query — and every role ODS declares
is held by someone, verified across nine releases from 2018 to 2026:

```sql
SELECT DISTINCT role_code, role_name FROM 'roles.parquet';   -- all 205
```

`orgs` and `orgs_all` share an identical schema, and that duplication is
deliberate: `orgs` is `orgs_all WHERE status = 'active'`, and exists so the
common case needs no filter and the naive query is the correct one. Parquet has
no views, so the alternative is every query carrying the filter.

## Naming

One rule: **`_code` is a reference, `_name` is a display label.** Every `_code`
resolves — to `orgs.ods_code`, to `roles.role_code`, or to a value carried
alongside it.

```
icb_code    = "QE1"                                    icb_name = "NHS LANCASHIRE AND SOUTH CUMBRIA ICB"
role_code   = "RO177"                                  role_name = "Prescribing Cost Centre"
role_codes  = ['RO177','RO76']                         (a list of references)
```

`_id` is different again: it identifies a specific *instance* rather than a
vocabulary entry. `role_id` is the ODS `uniqueRoleId` for one organisation's
holding of one role; `succession_id` identifies one succession.

Three columns carry no suffix because they reference nothing — they're values in
their own right: `name` (the row's own label, paired with `ods_code`), `status`
and `entity_type`.

### Decisions this settles

**A `_name` exists only where the label is worth carrying.** `_code` always
means a reference; it does not oblige a `_name` beside it.

**There is deliberately no `primary_role_name`.** The readable classification in
this schema is `role_names` — plural, and honest that an organisation does
several things. A singular readable column beside it would look like *the*
answer, and would be read as one, which is the failure this schema exists to
avoid. `primary_role_code` is safe next to `role_names` precisely because it is
opaque: nobody mistakes `RO177` for a description.

It would also be the least useful label available. The primary roles that occur
most often are `Social Care Site` (31,694), `Prescribing Cost Centre` (12,841)
and `Registered under Care Standards Act 2000` (5,477) — registers and
regulatory registrations rather than descriptions of the organisation.

Because `role_names` is positionally aligned with `role_codes`, ODS's own label
for the primary role is one expression away, with no join:

```sql
SELECT name, role_names[list_position(role_codes, primary_role_code)] AS primary_role_name
FROM 'orgs.parquet';
```

**There is no `category`.** It was a derived column that picked one role per
entity by a fixed precedence and copied its curated name. It added no vocabulary
— every value it could produce was already a `role_name` — and the precedence
was wrong wherever an entity genuinely does two things. `KING'S COLLEGE HOSPITAL
NHS FOUNDATION TRUST` was categorised `Hospice`, along with 74 other NHS trusts
that run hospice services. Filter on `role_names` instead; an organisation can be
more than one thing.

**`entity_type`, not `record_class`.** ODS calls it `orgRecordClass` with values
`RC1` and `RC2`, but "class" of what, and "record" describes the data model
rather than the thing. The distinction is about the entity, and the values are
`org` and `site`.

We translate `RC1`/`RC2` inline while keeping `RO177` raw for roles. Two values
translate inline; 205 need a lookup table. That's the line.

**Denormalise a tiny vocabulary; join for a rich entity.** `relationships` and
`successions` used to carry organisation names alongside the codes. Those names
were 54% of `relationships.parquet` by size, and if you are joining `orgs_all`
you usually want other fields from it anyway — so they went, and every code
resolves there with no exceptions.

`role_name` on `roles` is the same test read the other way: 205 values across
442,251 rows, dictionary-encoded, costing 96 KB — 3.8% of the file — so that
"which organisations hold this role" needs no join, and no separate vocabulary
table has to exist. `role_names` on `orgs` costs 0.9% for the same reason.

`rel_type_name` stays inline by the same test: nine values costing 0.06 MB under
dictionary encoding, and the name is the *only* thing a lookup would hold, so a
separate table would be a join for no gain. The names are carried **verbatim
from ODS**, uppercase and uncurated — `IS LOCATED IN THE GEOGRAPHY OF`. We
considered lowercasing them so they read as predicates in a sentence, and dropped
it: the shouty form is harmless, and it is one less transformation to justify.

**`relationships`, not `rels`.** It differed from `roles` by one letter, which
is a hazard when both appear in the same query. `org_roles` was renamed to
`roles` for the mirror of that reason: with a separate vocabulary table gone,
`roles` and `org_roles` would have been two nearly-identical names for the only
remaining role table.

**There is no `can_be_primary`.** It came from `<PrimaryRoleScope>` in the
manifest — ODS declaring which roles *their register* permits in the primary
slot. That is an artefact of the system of record, not a fact about NHS
organisations. It was also nearly redundant: no active organisation uses a
primary role outside the declared scope, so the only content it added was seven
roles ODS still declares but nothing active uses — all defunct structures like
`Primary Care Group`, abolished in 2002. For what is actually in use:

```sql
SELECT DISTINCT primary_role_code FROM 'orgs_all.parquet';
```

**`successions.parquet` is plural**, like every other table.

Dates follow ODS's own two-track model. `legal_*` is statutory existence,
`operational_*` is when it was actually running. **They routinely disagree** —
see [Point-in-time queries](#point-in-time-queries).

## `orgs.parquet` / `orgs_all.parquet`

35 columns.

### Identity and classification

| column | type | null | description |
| :--- | :--- | :--- | :--- |
| `ods_code` | VARCHAR | no | `"A82608"` |
| `name` | VARCHAR | no | `"SEDBERGH MEDICAL PRACTICE"` |
| `entity_type` | VARCHAR | no | `"org"` or `"site"` |
| `status` | VARCHAR | no | `"active"` or `"inactive"` |
| `role_codes` | VARCHAR[] | no | every role held, sorted and deduplicated — `['RO177','RO76']` |
| `role_names` | VARCHAR[] | no | the same roles, curated — `['Prescribing Cost Centre','GP Practice']` |
| `primary_role_code` | VARCHAR | no | `"RO177"` — the role ODS marks primary |

`role_names` is **positionally aligned** with `role_codes` — same length, same
order, each name the curated label for the code at that index. It is derived from
`role_codes`, so the two cannot disagree.

The order follows the *codes*, and the sort is **lexical on the code string, not
numeric** — so `RO197` sorts before `RO57` before `RO7`, and `RO177` before
`RO76`. That is why `RJZ` reads `['NHS Trust','Foundation Trust','Hospice']`
rather than by role number. The order carries no meaning: do not read
`role_names[1]` as the organisation's kind.

Filter on it to find organisations by what they do, with no join and no code
lookup:

```sql
SELECT ods_code, name, postcode FROM 'orgs.parquet'
WHERE list_contains(role_names, 'GP Practice');
```

An organisation can be several things at once, and this is the column that says
so. `RJZ` KING'S COLLEGE HOSPITAL NHS FOUNDATION TRUST carries
`['NHS Trust','Foundation Trust','Hospice']` — it is a trust *and* runs a
hospice. Narrow when you need to:

```sql
WHERE list_contains(role_names,'Hospice') AND NOT list_contains(role_names,'NHS Trust')
```

Use `role_codes` where you want a stable identifier rather than a label — codes
never change, curated names occasionally do.

### Succession

| column | type | null | description |
| :--- | :--- | :--- | :--- |
| `successor_codes` | VARCHAR[] | no | every organisation this one was eventually succeeded by |
| `predecessor_codes` | VARCHAR[] | no | every organisation that eventually became this one |

Both are the **transitive closure** of the succession graph, sorted and
deduplicated, and exact inverses: `X` appears in `predecessor_codes` of `Y`
precisely when `Y` appears in `successor_codes` of `X`. Empty array where there
is none, never NULL.

For the chain `0AF → 0CE → 0CY → YDDTR`:

```
0AF    successor_codes = ['0CE','0CY','YDDTR']
0CE    successor_codes = ['0CY','YDDTR']       predecessor_codes = ['0AF','0AN']
YDDTR  successor_codes = []                    predecessor_codes = ['0AF','0AJ','0AN','0CE','0CY']
```

Transitive rather than immediate, so mapping a historic code onto current
organisations is one line of SQL rather than a recursive query — which is the
step people get wrong:

```sql
-- I have code 0AF in a 2006 extract. What is it now?
SELECT ods_code, name FROM 'orgs.parquet'
WHERE list_contains(predecessor_codes, '0AF');
```

Note what does the work: `orgs.parquet` is active-only, so it returns exactly the
*live* organisations descended from `0AF`. Run it against `orgs_all.parquet` to
include ones that have since closed. There is deliberately no "terminal
successor" concept — 5,300 chains end at an inactive organisation and 58
organisations have both active and inactive endpoints, so "the live one it
became" isn't always defined.

Use `successions.parquet` for hop-by-hop detail and dates.

### Release identity

On **every table**, so releases can be concatenated without losing track of which
snapshot a row came from:

| column | type | null | description |
| :--- | :--- | :--- | :--- |
| `trud_release_date` | DATE | no | TRUD distribution release date — `2026-07-31` |

Constant within a file, so near-free under dictionary encoding. Sourced from the
TRUD release archive filename (e.g. `hscorgrefdataxml_data_7.0.0_20260731000001.zip`),
which is when NHS England cut the distribution release.

Inside the archive, the XML manifest header records a separate `publication_date`
(e.g. `2026-07-28`) and `publication_seq_num` (e.g. `4700`) representing the internal
ODS database export timestamp. Both are preserved in `_provenance.json` and the
Parquet key-value metadata. The TRUD release date in the archive filename is the
primary release identifier because it aligns with monthly release announcements and
distribution archives.

```sql
SELECT trud_release_date, count(*)
FROM read_parquet('releases/*/orgs.parquet', union_by_name = true)
GROUP BY 1 ORDER BY 1;
```

### Location and contact

| column | type | null | description |
| :--- | :--- | :--- | :--- |
| `address` | VARCHAR | yes | street address lines joined into one string |
| `town` | VARCHAR | yes | `"SEDBERGH"` |
| `county` | VARCHAR | yes | `"CUMBRIA"` |
| `postcode` | VARCHAR | yes | `"LA10 5DL"` — always space-separated |
| `country` | VARCHAR | yes | `"ENGLAND"` |
| `uprn` | VARCHAR | yes | Unique Property Reference Number |
| `telephone` | VARCHAR | yes | |
| `website` | VARCHAR | yes | |

ODS supplies up to three street address lines, which we join. That's deliberate
and loses nothing queryable: the lines have no positional semantics — one record
has `BROADMEADOWS` on line 2 and the village `SOUTH NORMANTON` on line 3 — while
the structure worth querying (`town`, `county`, `postcode`, `country`) is already
in its own columns.

Line 1 is present on every record, line 2 on 172,036 and line 3 on 40,627, and
they don't nest cleanly — a few hundred records carry line 3 without line 2. We
join the non-empty lines in order rather than assuming positions. The
organisation's own `name` is separate and never part of the address.

`telephone` and `website` are the only contact details ODS publishes. Its
`<Contact>` elements carry just two types across all 305,541 organisations,
`tel` and `http`. There is no email or fax data to carry.

### Derived hierarchy

Resolved relationships, flattened for convenience. A NULL means **ODS records no
such relationship for this entity**, not that resolution failed.

| column | type | example |
| :--- | :--- | :--- |
| `commissioner_code` / `commissioner_name` | VARCHAR | `"QE1"` |
| `parent_code` / `parent_name` | VARCHAR | `"01K"` |
| `pcn_code` / `pcn_name` | VARCHAR | `"U59980"` / `"WESTERN DALES PCN"` |
| `trust_code` / `trust_name` | VARCHAR | |
| `icb_code` / `icb_name` | VARCHAR | |
| `region_code` / `region_name` | VARCHAR | `"Y62"` |

Fill rates on 216,886 active rows: `region_code` 103,787 · `trust_code` 40,148 ·
`icb_code` 31,380 · `pcn_code` 7,584. The spread reflects the entity mix — most
entities aren't GP practices, so most have no PCN.

Use `relationships.parquet` if you need the relationship's own dates or status.

### Dates

| column | type | null | description |
| :--- | :--- | :--- | :--- |
| `legal_start` | DATE | yes | statutory start |
| `legal_end` | DATE | yes | statutory end; NULL while current |
| `operational_start` | DATE | yes | when it began operating |
| `operational_end` | DATE | yes | when it stopped (note: 603 active records in source data carry operational end dates; filter by status = 'active' to select active organisations) |
| `last_changed` | DATE | yes | when ODS last modified this record |

## `roles.parquet`

One row per organisation per role, including **inactive** roles, which is what
makes role history reconstructable from a single release. This is the role table;
there is no separate vocabulary table.

| column | type | null | description |
| :--- | :--- | :--- | :--- |
| `ods_code` | VARCHAR | no | joins to `orgs.ods_code` |
| `role_code` | VARCHAR | no | `"RO76"` |
| `role_name` | VARCHAR | no | curated — `"GP Practice"` |
| `role_id` | VARCHAR | no | ODS `uniqueRoleId` — identifies this holding |
| `is_primary` | BOOLEAN | no | is this the entity's primary role |
| `role_status` | VARCHAR | no | of the *role holding*, not the organisation |
| `legal_start` / `legal_end` | DATE | yes | |
| `operational_start` / `operational_end` | DATE | yes | |
| `trud_release_date` | DATE | no | TRUD distribution release date |

All 131,871 inactive rows carry an `operational_end` and every row carries a
start, so "which roles did this organisation hold on date D?" is answerable from
this table alone.

**One organisation can hold the same role more than once** — 663 pairs do, as
separate holdings with their own dates. So a filtered join against this table
returns duplicate organisations unless you deduplicate: filtering on
`role_name = 'Domiciliary Care'` yields 16,726 rows for 16,157 organisations. Use
`orgs.role_names` when you want organisations, and this table when you want
holdings and their dates.

### The role vocabulary

205 role codes, all of them held by at least one organisation:

```sql
SELECT DISTINCT role_code, role_name FROM 'roles.parquet' ORDER BY role_name;
```

Names are curated: typos fixed, abbreviations expanded, casing normalised, with a
justification recorded for every substantive change. 186 of the 205 differ from
the source only by case; 19 differ substantively, and those repair real defects —
`RO258` is truncated at 50 characters in the source, `RO215` misspells
`MANAGEMENT`.

This is a deliberate exception to carrying values verbatim, and the reason is
what the column is for: **`name` and `rel_type_name` are labels you read;
`role_name` is a key you type.** `role_names` is the documented query interface,
and a controlled vocabulary that users type has different requirements from a
label that is only displayed. It is also why the curation trends terser — `ICB`,
`PCN` — rather than prettier.

The exact mapping, and the justification for every substantive change, are in
`data/role_names.json` at the commit recorded as `tool_git_sha` in
`_provenance.json`. The file is not shipped in the release because the commit
already pins it — the same reasoning that keeps library versions out of
provenance.

**A file that transforms the data ships with the release and is hashed, unless
the recorded commit already pins it. A file that is a display opinion stays in
the binary and never ships.**

## `relationships.parquet`

One row per relationship, active and inactive. Organisation names come from a
join; the relationship type is carried inline.

| column | type | null | description |
| :--- | :--- | :--- | :--- |
| `rel_id` | VARCHAR | no | ODS `uniqueRelId` — identifies this relationship |
| `source_code` | VARCHAR | no | joins to `orgs.ods_code` |
| `target_code` | VARCHAR | no | joins to `orgs.ods_code` |
| `rel_type_code` | VARCHAR | no | `"RE5"` |
| `rel_type_name` | VARCHAR | no | `"IS LOCATED IN THE GEOGRAPHY OF"` — verbatim from ODS |
| `rel_status` | VARCHAR | no | of the *relationship* |
| `legal_start` / `legal_end` | DATE | yes | |
| `operational_start` / `operational_end` | DATE | yes | |
| `trud_release_date` | DATE | no | TRUD distribution release date |

Relationships are directional: `source_code` holds the relationship *to*
`target_code`. `rel_id` is what lets you tell a continuing relationship from a
new one that replaced it, across releases.

The nine type names are ODS's own, uppercase:

```sql
SELECT s.name, r.rel_type_name, t.name
FROM 'relationships.parquet' r
JOIN 'orgs_all.parquet' s ON s.ods_code = r.source_code
JOIN 'orgs_all.parquet' t ON t.ods_code = r.target_code
WHERE r.source_code = '00C';
-- NHS DARLINGTON CCG | IS LOCATED IN THE GEOGRAPHY OF | DURHAM, DARLINGTON AND TEES AREA TEAM
```

## `successions.parquet`

One row per succession — the immediate, hop-by-hop edges. 11,568 rows. Codes
only, as above.

| column | type | null | description |
| :--- | :--- | :--- | :--- |
| `succession_id` | VARCHAR | no | ODS `uniqueSuccId` |
| `predecessor_code` | VARCHAR | no | the organisation that was superseded |
| `successor_code` | VARCHAR | no | the organisation that took over |
| `legal_start` | DATE | no | the date the succession took effect |
| `trud_release_date` | DATE | no | TRUD distribution release date |

Three details, all verified against the full XML:

**ODS states successions from both directions.** 11,538 records say "X is my
predecessor" and 1,760 say "Y is my successor". Both are normalised into one
forward edge here. Where a succession is stated from both ends — 1,730 of them —
**both records carry the same `uniqueSuccId`**, so deduplication is exact:
13,298 records become 11,568 successions.

**There is no end date, because a succession is an event.** Every record carries
`<Date><Type value="Legal"/><Start .../></Date>` and none carries an `End`.
Roles and relationships are states with duration, so they get both. A succession
happens on a day.

**Both endpoints can be active or inactive.** 304 active organisations have
successors, and 5,300 chains end at an organisation that has itself closed.

```sql
-- CLWYD became five Welsh unitary authorities in 1996
SELECT successor_code, legal_start FROM 'successions.parquet'
WHERE predecessor_code = '001';
```

For the resolved view — everything an organisation eventually became, without
walking the chain yourself — use `successor_codes` and `predecessor_codes` on
`orgs`.

## Point-in-time queries

`orgs.parquet` is *active as of the release date*. To ask what was open on an
earlier date, query `orgs_all.parquet` with a date predicate:

```sql
SELECT ods_code, name
FROM 'orgs_all.parquet'
WHERE operational_start <= DATE '2019-03-31'
  AND (operational_end IS NULL OR operational_end > DATE '2019-03-31');
```

Three things to know before relying on it:

1. **Use `operational_*`.** `legal_*` is only populated for organisations created
   by statute — 14% of rows — so a predicate on it silently drops the rest. See
   [Use the operational dates](./queries.md#use-the-operational-dates).
2. **A snapshot taken today won't reproduce one taken in 2019.** Entities are
   registered, closed, reopened and sometimes deleted between releases, and ODS
   backfills end dates as information arrives.
3. **Name and address history isn't in a single release.** Each release carries
   one current name per entity, so reconstructing a change means comparing
   releases.

Point 2 is why releases are pinned and immutable. If you need the state as ODS
knew it in 2019, you need the 2019 release, not a 2019 filter over today's —
[measured here](./queries.md#which-release-should-you-use).

Points 2 and 3 turn into features if you keep the releases. Closures,
reopenings, renames and reparenting are all recoverable
[across an archive](./queries.md#across-releases).

## Frictionless Data Package

`datapackage.json` describes this release's tables in the Frictionless Table
Schema format. The three tables without list columns validate with the
Frictionless framework; `orgs` and `orgs_all` use native Parquet list columns for
`role_codes`, `role_names`, `successor_codes` and `predecessor_codes`, which
Table Schema's flat-cell model does not cover. For querying, the Parquet files
are self-describing — use DuckDB, Polars or Arrow directly.

## Provenance

Every release ships `_provenance.json` and `SHA256SUMS`, recording the TRUD
archive it derives from, its verified SHA-256, the ODS publication date and
sequence number, and the commit that built it (`tool_git_sha`) — enough to
rebuild byte-identical output. Hashes are uppercase throughout, matching how TRUD
publishes theirs. `ods cite` renders it as a citation.

Parquet key-value metadata carries the same facts, so a file separated from its
directory is still self-describing:

```sql
SELECT key, value FROM parquet_kv_metadata('orgs.parquet');
```
