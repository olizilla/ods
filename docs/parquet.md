# Parquet schemas

The tables `ods make` produces from the NHS TRUD ODS XML release. Worked
examples, including how to query several releases at once, are in
[queries.md](./queries.md).

> **Pre-1.0.** This is the schema **as it will be at v1**. Several renames and
> the succession rework are agreed but not yet implemented — see
> `.agents/briefs/schema-freeze-workplan.md`. We publish v1 of the data once
> we're happy with this shape, and it's fixed from then on.

## The tables

| file | grain | rows (7.0.0) | purpose |
| :--- | :--- | ---: | :--- |
| `orgs.parquet` | one per active organisation or site | 216,886 | the main analytical table |
| `orgs_all.parquet` | one per organisation or site, active **and** inactive | 305,541 | historical work |
| `org_roles.parquet` | one per organisation per role | 442,251 | which roles an entity holds, and when |
| `roles.parquet` | one per role code | 205 | the role vocabulary |
| `relationships.parquet` | one per relationship | 662,558 | how entities relate |
| `successions.parquet` | one per succession | 11,568 | mergers, splits, renames |

`orgs` and `orgs_all` share an identical schema. `orgs` is
`orgs_all WHERE status = 'active'`, and exists so the common case needs no
filter and the naive query is the correct one.

`orgs` answers nearly everything about the NHS as it is today.
`relationships` and `successions` are for deeper work and are designed to be
joined against `orgs_all` rather than read alone.

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

Four columns carry no suffix because they reference nothing — they're values in
their own right: `name` (the row's own label, paired with `ods_code`), `status`,
`entity_type` and `category`.

### Decisions this settles

**Everything is suffixed, even where a name column doesn't exist.**
`primary_role_code` has no `primary_role_name` beside it, and is still
suffixed. Consistency is worth more than brevity here: you can guess any column
name in this schema without looking it up.

**There is deliberately no `primary_role_name`.** `category` exists *because*
the primary role name misleads — 9,372 GP practices are registered as
`RO177 Prescribing Cost Centre`, which is accurate and useless. Two readable
classification columns side by side would give no signal about which to trust.
`primary_role_code` is safe next to `category` precisely because it's opaque:
nobody mistakes `RO177` for a description. Join `roles.parquet` if you
specifically want ODS's own label.

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

`rel_type_name` stays inline by the same test: nine values costing 0.06 MB under
dictionary encoding, and the name is the *only* thing a lookup would hold, so a
separate table would be a join for no gain. The names are carried **verbatim
from ODS**, uppercase and uncurated — `IS LOCATED IN THE GEOGRAPHY OF`. We
considered lowercasing them so they read as predicates in a sentence, and dropped
it: the shouty form is harmless, and it is one less transformation to justify.

**`relationships`, not `rels`.** It differed from `roles` by one letter, which
is a hazard when both appear in the same query.

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
| `primary_role_code` | VARCHAR | no | `"RO177"` — joins to `roles.role_code` |
| `role_codes` | VARCHAR[] | no | every role held, sorted and deduplicated — `['RO177','RO76']` |
| `category` | VARCHAR | no | derived, opinionated — `"GP Practice"` |

`category` is the one column that isn't ODS data. The ODS primary role sometimes
describes the *register* rather than the entity. Five rules cover the cases where
it actively misleads; everything else falls back to the curated name of the
primary role. The rules and their justifications ship in `category_rules.json`
with every release, hashed like everything else, so you can disagree and
recompute.

For anything more precise than `category`, filter on `role_codes`:

```sql
SELECT * FROM 'orgs.parquet' WHERE list_contains(role_codes, 'RO76');
```

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
| `operational_end` | DATE | yes | when it stopped; NULL while current |
| `last_changed` | DATE | yes | when ODS last modified this record |

## `org_roles.parquet`

One row per organisation per role, including **inactive** roles, which is what
makes role history reconstructable from a single release.

| column | type | null | description |
| :--- | :--- | :--- | :--- |
| `ods_code` | VARCHAR | no | joins to `orgs.ods_code` |
| `role_code` | VARCHAR | no | joins to `roles.role_code` |
| `role_id` | VARCHAR | no | ODS `uniqueRoleId` — identifies this holding |
| `is_primary` | BOOLEAN | no | is this the entity's primary role |
| `role_status` | VARCHAR | no | of the *role holding*, not the organisation |
| `legal_start` / `legal_end` | DATE | yes | |
| `operational_start` / `operational_end` | DATE | yes | |

All 131,871 inactive rows carry an `operational_end` and every row carries a
start, so "which roles did this organisation hold on date D?" is answerable from
this table alone.

## `roles.parquet`

The role vocabulary. 205 rows, of which 97 can be a primary role.

| column | type | null | description |
| :--- | :--- | :--- | :--- |
| `role_code` | VARCHAR | no | `"RO177"` |
| `role_name` | VARCHAR | no | curated — `"Prescribing Cost Centre"` |
| `can_be_primary` | BOOLEAN | no | whether ODS declares it primary-capable |

Names are curated: typos fixed, abbreviations expanded, casing normalised, with a
justification recorded for every substantive change.

`can_be_primary` comes from the `<PrimaryRoleScope>` declaration in the release
manifest, so it's ODS's own statement rather than something inferred from usage.
That matters: 4 roles are used as primary by exactly one organisation, so a
computed version would flip to `false` the month that organisation closed,
without anything about the role changing.

**90 roles are declared, and 97 appear as primary in the data.** The extra 7 —
`RO106`, `RO109`, `RO111`, `RO114`, `RO144`, `RO149`, `RO171` — are used only by
*inactive* organisations. They're historical, and ODS has since dropped them from
the current scope. So for historical work, take the roles actually in use rather
than filtering on this column:

```sql
SELECT DISTINCT primary_role_code FROM 'orgs_all.parquet';
```

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

## Provenance

Every release ships `_provenance.json` and `SHA256SUMS`, recording the TRUD
archive it derives from, its verified SHA-256, the ODS publication date and
sequence number, and the tool and library versions used — enough to rebuild
byte-identical output. Hashes are uppercase throughout, matching how TRUD
publishes theirs. `ods cite` renders it as a citation.

Parquet key-value metadata carries the same facts, so a file separated from its
directory is still self-describing:

```sql
SELECT key, value FROM parquet_kv_metadata('orgs.parquet');
```
