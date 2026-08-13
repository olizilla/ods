# Parquet schemas

The tables `ods make` produces from the NHS TRUD ODS XML release.

> **Pre-1.0.** This documents the schema **as it will be at first publication**,
> including changes agreed in the schema freeze but not yet implemented:
> `rel_id`, `succession_id`, `succession_type` and succession dates are new, and
> `roles.name` becomes `role_name`. See
> `.agents/briefs/schema-freeze-workplan.md`. Once a release is published and
> citable, the shape is fixed.

---

## The tables

| file | grain | rows (7.0.0) | purpose |
|---|---|---|---|
| `orgs.parquet` | one per active organisation or site | 216,886 | the main analytical table |
| `orgs_all.parquet` | one per organisation or site, active **and** inactive | 305,541 | historical work |
| `org_roles.parquet` | one per organisation per role | 442,251 | which roles an entity holds, and when |
| `roles.parquet` | one per role code | 205 | the role vocabulary |
| `rels.parquet` | one per relationship | 662,558 | how entities relate |
| `successors.parquet` | one per succession link | 90,913 | mergers, splits, renames |

`orgs` and `orgs_all` share an identical schema. `orgs` is
`orgs_all WHERE status = 'active'`, and exists so the common case needs no
filter.

**Join key:** `ods_code` throughout, except `rels`, which uses `source_code` and
`target_code`.

---

## Naming conventions

These are recorded because the `_code` suffix looks inconsistent at a glance and
is not. Reviewers reliably spot the apparent inconsistency and propose
"fixing" it; this section is the answer.

### `_code` does two different jobs

**As a disambiguator** — there is a sibling name column, and the suffix tells
them apart:

```
icb   = "NHS LANCASHIRE AND SOUTH CUMBRIA ICB"     icb_code   = "QE1"
trust = "…"                                        trust_code = "…"
```

**As a type annotation** — there is no sibling, and the suffix says *"this value
is an identifier of the ODS kind"*: `ods_code`, `role_code`, `rel_type_code`.
There is no bare `ods` or `role` column for these to be distinguished from.

### `_id` versus `_code`

`_code` names an entry in a *vocabulary* (`role_code` = `"RO177"`, one of 205).
`_id` identifies a specific *instance* of it (`role_id` = the ODS
`uniqueRoleId` for one organisation's holding of one role).

### The test: would a practitioner say it aloud?

"ODS code" ✓ · "role code" ✓ · "ICB code" ✓ — all real phrases.
"role codes" ✗ · "primary role code" ✗ — grammatical, but nobody says them.

A name should be a phrase, not a construction.

### Decisions this settles

**`ods_code`, not `OrgId`.** `OrgId` is the XML serialisation's element name, and
it is misleading here: the same element carries sites as well as organisations,
and roughly a third of records are sites. "ODS code" is the practitioner's term,
covers both, and is unambiguous. Trace-to-source governs *facts*; element names
are a transport detail.

**`primary_role`, not `primary_role_code`.** The disambiguator rule does not
apply — there is no `primary_role_name` column to distinguish from — so the
suffix would be pure length, and it fails the say-it-aloud test.

**`roles`, not `role_codes`.** Same reasoning. `roles` is a word; `role_codes`
is a construction, and this is a common grouping and filtering column where
brevity is worth most.

**`rels` uses `source_code` / `target_code`, not `ods_code`.** A relationship
has two organisation references and cannot call either one `ods_code` without
ambiguity. This creates a real discontinuity — `orgs.ods_code` joins to
`org_roles.ods_code` by the same name but to `rels.source_code` by a different
one. It is accepted deliberately: no suffix scheme removes it, because the
underlying shape genuinely differs.

### Elsewhere

`status` is qualified per table — `role_status`, `rel_status` — because an active
organisation can hold an inactive role, and three columns named `status` across
joined tables is a footgun rather than a convenience.

Dates follow ODS's own two-track model. `legal_*` is the statutory existence of
the entity; `operational_*` is when it was actually running. **They routinely
disagree**, and which one you want depends on the question — see
[Point-in-time queries](#point-in-time-queries).

---

## `orgs.parquet` / `orgs_all.parquet`

32 columns.

### Identity and classification

| column | type | null | description |
|---|---|---|---|
| `ods_code` | VARCHAR | no | ODS code — `"A82608"` |
| `record_class` | VARCHAR | no | `"org"` or `"site"` |
| `status` | VARCHAR | no | `"active"` or `"inactive"` |
| `primary_role` | VARCHAR | no | ODS code of the primary role — `"RO177"` |
| `roles` | VARCHAR[] | no | every role code held, **sorted** and deduplicated — `['RO177','RO76']` |
| `category` | VARCHAR | no | **derived, opinionated** — `"GP Practice"`. See below |
| `name` | VARCHAR | no | `"SEDBERGH MEDICAL PRACTICE"` |

### Release identity

Present on **every table**, so releases can be concatenated without losing track
of which snapshot a row came from:

| column | type | null | description |
|---|---|---|---|
| `publication_seq_num` | VARCHAR | no | ODS's own monotonic release counter — `"4700"` |
| `publication_date` | DATE | no | when NHS generated the data — `2026-07-28` |

Both are constant within a file and cost almost nothing under dictionary
encoding. They are deliberately `publication_*` rather than `release_date`: the
TRUD *distribution* date (2026-07-31) differs from the ODS *publication* date
(2026-07-28), and stamping the ambiguous one on 300,000 rows would spread that
confusion rather than contain it.

```sql
-- concatenate releases and keep track of provenance per row
SELECT publication_date, count(*)
FROM read_parquet('releases/*/orgs.parquet', union_by_name = true)
GROUP BY 1 ORDER BY 1;
```

`category` is the one column that is not ODS data. It answers "what kind of thing
is this?" in a single readable string, because the ODS primary role sometimes
describes the *register* rather than the entity — GP practices are registered as
`RO177 Prescribing Cost Centre`. Five rules cover the cases where the primary
role actively misleads; everything else falls back to the curated name of the
primary role. The rules and their justifications ship in `category_rules.json`
alongside the data, hashed like every other artifact.

For anything more precise than `category`, filter on `roles`:

```sql
SELECT * FROM 'orgs.parquet' WHERE list_contains(roles, 'RO76');
```

### Location and contact

| column | type | null | description |
|---|---|---|---|
| `address` | VARCHAR | yes | address lines, joined into one string |
| `town` | VARCHAR | yes | `"SEDBERGH"` |
| `county` | VARCHAR | yes | `"CUMBRIA"` |
| `postcode` | VARCHAR | yes | `"LA10 5DL"` — always space-separated |
| `country` | VARCHAR | yes | `"ENGLAND"` |
| `uprn` | VARCHAR | yes | Unique Property Reference Number |
| `telephone` | VARCHAR | yes | |
| `website` | VARCHAR | yes | |

### Derived hierarchy

Each pair is a resolved relationship, flattened for convenience. All are
nullable, and a NULL means **ODS records no such relationship for this entity**,
not that resolution failed.

| column | type | description |
|---|---|---|
| `commissioner` / `commissioner_code` | VARCHAR | commissioning body |
| `parent` / `parent_code` | VARCHAR | immediate administrative parent |
| `pcn` / `pcn_code` | VARCHAR | Primary Care Network |
| `trust` / `trust_code` | VARCHAR | overarching trust |
| `icb` / `icb_code` | VARCHAR | Integrated Care Board |
| `region` / `region_code` | VARCHAR | NHS England regional directorate |

Fill rates on 216,886 active rows: `region_code` 103,787 · `trust_code` 40,148 ·
`icb_code` 31,380 · `pcn_code` 7,584. The spread reflects the entity mix — most
entities are not GP practices and so have no PCN.

Use `rels.parquet` if you need the relationship's own dates or status; these
columns are a convenience projection.

### Dates

| column | type | null | description |
|---|---|---|---|
| `legal_start` | DATE | yes | statutory start |
| `legal_end` | DATE | yes | statutory end; NULL while current |
| `operational_start` | DATE | yes | when it began operating |
| `operational_end` | DATE | yes | when it stopped; NULL while current |
| `last_change_date` | DATE | yes | last modification by the publisher |

---

## `org_roles.parquet`

One row per organisation per role — including **inactive** roles, which is what
makes role history reconstructable from a single release.

| column | type | null | description |
|---|---|---|---|
| `ods_code` | VARCHAR | no | joins to `orgs.ods_code` |
| `role_code` | VARCHAR | no | joins to `roles.role_code` — `"RO177"` |
| `role_id` | VARCHAR | no | ODS `uniqueRoleId` — identifies this holding |
| `is_primary` | BOOLEAN | no | is this the entity's primary role |
| `role_status` | VARCHAR | no | `"active"` or `"inactive"` — of the *role holding*, not the organisation |
| `legal_start` / `legal_end` | DATE | yes | |
| `operational_start` / `operational_end` | DATE | yes | |

All 131,871 inactive rows carry an `operational_end`, and every row carries a
start — so "which roles did this organisation hold on date D?" is answerable
from this table alone.

---

## `roles.parquet`

The role vocabulary. 205 rows, of which 97 can be a primary role.

| column | type | null | description |
|---|---|---|---|
| `role_code` | VARCHAR | no | `"RO177"` |
| `role_name` | VARCHAR | no | curated display name — `"Prescribing Cost Centre"` |
| `can_be_primary` | BOOLEAN | no | whether ODS ever uses it as a primary role |

Names are curated: typos fixed, abbreviations expanded, casing normalised. The
curation file records a justification for every substantive change.

---

## `rels.parquet`

One row per relationship, active and inactive.

| column | type | null | description |
|---|---|---|---|
| `rel_id` | VARCHAR | no | ODS `uniqueRelId` — identifies this relationship |
| `source` | VARCHAR | yes | `"NHS DARLINGTON CCG"` |
| `source_code` | VARCHAR | no | `"00C"` — joins to `orgs.ods_code` |
| `target` | VARCHAR | yes | `"DURHAM, DARLINGTON AND TEES AREA TEAM"` |
| `target_code` | VARCHAR | no | `"Q45"` |
| `rel_type` | VARCHAR | no | `"is located in the geography of"` |
| `rel_type_code` | VARCHAR | no | `"RE5"` |
| `rel_status` | VARCHAR | no | `"active"` or `"inactive"` — of the *relationship* |
| `legal_start` / `legal_end` | DATE | yes | |
| `operational_start` / `operational_end` | DATE | yes | |

Relationships are directional: `source` holds the relationship *to* `target`.

---

## `successors.parquet`

Mergers, splits and renames. **Many-to-many** — an organisation may have several
successors, and several predecessors.

| column | type | null | description |
|---|---|---|---|
| `ods_code` | VARCHAR | no | the superseded entity — `"001"` |
| `name` | VARCHAR | no | `"CLWYD"` |
| `succession_id` | VARCHAR | no | ODS `uniqueSuccId` — identifies this link |
| `succession_type` | VARCHAR | no | `"Successor"` or `"Predecessor"` |
| `successor_code` | VARCHAR | no | `"016"` |
| `successor` | VARCHAR | no | `"CONWY UA"` |
| `succession_chain` | VARCHAR | no | `"001 -> 016"` |
| `legal_start` / `legal_end` | DATE | yes | when the succession took effect |
| `operational_start` / `operational_end` | DATE | yes | |

90,913 rows across 88,655 organisations, so most successions are one-to-one, but
the multi-successor cases are real and matter:

```sql
SELECT * FROM 'successors.parquet' WHERE ods_code = '001';
-- 001 CLWYD → 016 CONWY UA
-- 001 CLWYD → 018 DENBIGHSHIRE UA
-- 001 CLWYD → 020 FLINTSHIRE UA
```

Taking only the first successor silently drops two thirds of that population.
Unresolved mergers are a leading source of error in longitudinal analysis of
NHS organisational data — always aggregate across the full set.

---

## Point-in-time queries

`orgs.parquet` is *active as of the release date*. To ask what was open on some
earlier date, query `orgs_all.parquet` with a date predicate:

```sql
SELECT ods_code, name
FROM 'orgs_all.parquet'
WHERE operational_start <= DATE '2019-03-31'
  AND (operational_end IS NULL OR operational_end > DATE '2019-03-31');
```

Three things to know before relying on this:

1. **`legal_*` and `operational_*` routinely disagree.** Pick deliberately.
   Operational dates usually match what a patient would have experienced; legal
   dates match statutory records.
2. **Closure dates are applied retrospectively.** A snapshot taken today will
   *not* reproduce a snapshot taken in 2019 — ODS backfills end dates as
   information arrives.
3. **Name and address history is not in a single release.** Each release carries
   one current name per entity. Reconstructing when a name changed requires
   comparing releases.

Point 2 is the reason releases are pinned and immutable. If you need the state
as ODS knew it in 2019, you need the 2019 release, not a 2019 filter over
today's.

---

## Provenance

Every release ships `_provenance.json` and `SHA256SUMS`. The provenance records
the TRUD archive it derives from, its verified SHA-256, the ODS publication date
and sequence number, and the tool and library versions used — enough to
reproduce the byte-identical output. `ods cite` renders it as a citation.

Parquet key-value metadata carries the same facts, so a file separated from its
directory is still self-describing:

```sql
SELECT key, value FROM parquet_kv_metadata('orgs.parquet');
```
