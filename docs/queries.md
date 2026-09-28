# Queries

Some interesting queries with real output. The schema and the reasoning behind it is in [parquet.md].

You can point `duckdb` at the parquet files and go. Local-first! All examples here show the output given from the `2026-08-28` release. `orgs.parquet` holds every organisation, closed ones too, so most queries below filter with `WHERE status = 'active'`.

`status = 'active'` alone isn't the same question as "is this open": NHS keeps a legally dissolved organisation `active` for a migration window that's meant to be six months and often runs years over (see [nhs.md](./nhs.md#status-active-and-open)). What `ods find` and `ods role` actually filter on, and what a query should use if it means the same thing, is:

```sql
WHERE status = 'active' AND (legal_end IS NULL OR legal_end > trud_release_date)
```

Compare to `trud_release_date`, the column, never to `current_date`: a query against a dated, immutable release should answer the same way whenever you run it. Comparing to today's date instead makes the same query drift out of date the moment it's written, and answer differently on the same file next year.

```console
$ duckdb -c "SELECT ods_code, name, role_names FROM 'ods_data/current/orgs.parquet' WHERE status = 'active' AND town = 'SEDBERGH'"
┌───────────┬──────────────────────────────────────────┬────────────────────────────────────────┐
│ ods_code  │                   name                   │               role_names               │
├───────────┼──────────────────────────────────────────┼────────────────────────────────────────┤
│ 8GJ58     │ PARKER M JUNE (ACUPUNCURIST)             │ [Non-NHS Organisation]                 │
│ A82608    │ SEDBERGH MEDICAL PRACTICE                │ [Prescribing Cost Centre, GP Practice] │
│ A82608001 │ DR LUMB W & PARTNER                      │ [Branch Surgery]                       │
│ D2E8H     │ AP SD THIRTEEN LIMITED                   │ [Pharmacy Headquarter]                 │
│ EE112233  │ SEDBERGH PRIMARY SCHOOL                  │ [School, Community School]             │
│ EE112331  │ DENT COFE VOLUNTARY AIDED PRIMARY SCHOOL │ [School, Voluntary Aided School]       │
│ EE112451  │ SEDBERGH SCHOOL                          │ [School, Other Independent School]     │
│ EE137269  │ SETTLEBECK SCHOOL                        │ [School, Academy Converter]            │
│ FLG02     │ ALLIED PHARMACY SEDBERGH                 │ [Pharmacy]                             │
│ RNN88     │ SEDBERGH HEALTH CENTRE                   │ [NHS Trust Site]                       │
│ RW5OX     │ SEDBURGH MEDICAL CENTRE                  │ [NHS Trust Site]                       │
│ RX796     │ SEDBURGH AMBULANCE STATION               │ [NHS Trust Site]                       │
│ V25604    │ MAIN STREET DENTAL SURGERY               │ [General Dental Practice]              │
│ VN6C2     │ PRIVATE PERSONAL ASSISTANCE LIMITED      │ [Social Care Site, Domiciliary Care]   │
└───────────┴──────────────────────────────────────────┴────────────────────────────────────────┘
```

The numbers here come from the release `current` points at, `2026-08-28`. Yours
will differ if you're pinned elsewhere... `ods pull --list` tells you what you've
got. Single-release queries come first, then the ones that need an archive of
releases.

## The source data has issues

In the example above `SEDBURGH MEDICAL CENTRE` is misspelled in the source data. 
If you find more errors, open issues so we can report them upstream.

Other things worth knowing before you query:

- **Use `role_names` to find out what something is.** The `primary_role_code` describes 
  GP practices as `RO177 Prescribing Cost Centre`, an administrative bucket rather than its function.
  `role_names` contains the curated, readable names for every role held.
- **Use `role_codes` to find all the official buckets an entity is in.** It's already the
  current, deduplicated set — no join needed. Reach for `roles.parquet` only for a
  holding's *history* (dates, status), and see [why joining it to filter on a role
  can double-count](#filtering-on-a-role-without-duplicating-rows).
- **You can filter on `country` if you need to.** ODS covers the UK and dependencies, containing six distinct country values: `ENGLAND` (209,527), `WALES` (5,746), `SCOTLAND` (1,178), `NORTHERN IRELAND` (354), `ISLE OF MAN` (200), and `CHANNEL ISLANDS` (54), counting the active organisations.
- **Expect a list of successors, not one.** ODS code `001` has five. Following just
  the first is a dead end, and unresolved mergers are a leading source of error in
  longitudinal analysis of NHS data.
- **[Prefer `operational` dates](#use-the-operational-dates)** to find when a thing was active. `legal_*` dates can be
  different to "when a site was operational" and are often not available.
- **[Join on `ods_code`](#names-are-not-identifiers).** Names aren't unique identifiers. 1,679 active entities are called `DENTAL SURGERY`.
  [docs/parquet.md](./parquet.md#joining-the-tables) lists the join key for every table.
- **[Use the latest release for history](#which-release-should-you-use)**, and the
  release from the time if you need what ODS knew then. Entities get registered,
  closed, reopened and sometimes deleted outright between releases.
- **[Compare releases to see a name or address change](#name-and-address-history).**
  Each release only carries the current name for an entity.

## What kind of thing is it

```sql
SELECT role, count(*) AS n
FROM (SELECT unnest(role_names) AS role
      FROM 'ods_data/current/orgs.parquet'
      WHERE status = 'active' AND record_class = 'org')
GROUP BY 1 ORDER BY n DESC LIMIT 8;
```
```
┌─────────────────────────┬───────┐
│          role           │   n   │
│         varchar         │ int64 │
├─────────────────────────┼───────┤
│ Social Care Site        │ 31738 │
│ School                  │ 25188 │
│ Social Care Provider    │ 18681 │
│ Non-NHS Organisation    │ 17368 │
│ Domiciliary Care        │ 16027 │
│ Care Home               │ 16021 │
│ Prescribing Cost Centre │ 12813 │
│ Pharmacy                │ 11186 │
└─────────────────────────┴───────┘
```

Because `role_names` is positionally aligned with `role_codes`, you can derive
the primary role name without a join:

```sql
SELECT name, role_names[list_position(role_codes, primary_role_code)] AS primary_role_name
FROM 'ods_data/current/orgs.parquet'
WHERE status = 'active'
LIMIT 5;
```

Most of ODS isn't what you'd picture as the NHS. Schools and care homes
outnumber NHS sites, because ODS registers everyone who exchanges data with the
NHS, not just the bits the NHS owns.

Filter on `role_names` or `role_codes`. about 93k active entities have more than one role:

```sql
SELECT len(role_codes) AS roles, count(*) AS entities
FROM 'ods_data/current/orgs.parquet' WHERE status = 'active' GROUP BY 1 ORDER BY 1;
```
```
┌───────┬──────────┐
│ roles │ entities │
│ int64 │  int64   │
├───────┼──────────┤
│     1 │   124169 │
│     2 │    92290 │
│     3 │      597 │
│     4 │        3 │
└───────┴──────────┘
```

## ODS has no hospital concept

A hospital building, its A&E and its fertility unit are all the same role,
`RO198 NHS Trust Site` — there's no `Hospital` role to filter on.

```sql
SELECT count(*) AS all_sites,
       count(*) FILTER (WHERE name LIKE '%HOSPITAL%') AS named_hospital
FROM 'ods_data/current/orgs.parquet' WHERE status = 'active' AND list_contains(role_codes, 'RO198');
-- 38256 | 3701
```

Only 1 in 10 sites has "hospital" in its name. To find a hospital's whole
estate, find its trust and search that trust's sites via `relationships.parquet`:

```sql
SELECT s.ods_code, s.name
FROM 'ods_data/current/relationships.parquet' r
JOIN 'ods_data/current/orgs.parquet' s ON r.source_code = s.ods_code
WHERE s.status = 'active' AND r.target_code = 'RRV' AND r.rel_code = 'RE6' AND r.rel_status = 'active';
```

## RO177 is England's prescribing register, not the UK's

```sql
SELECT count(*) AS scottish_gp_practices,
       count(*) FILTER (WHERE list_contains(role_codes, 'RO177')) AS also_ro177
FROM 'ods_data/current/orgs.parquet' WHERE status = 'active' AND list_contains(role_codes, 'RO227');
-- 1011 | 0
```

Not one of the 1,011 Scottish GP practices (`RO227`) holds `RO177 Prescribing
Cost Centre` — Scotland runs its own register. 70 of them hold `RO72 Other
Prescribing Cost Centre` instead. So `role_codes` containing `RO177` means
"English prescribing cost centre", not "prescribes medicine" — a UK-wide
question needs `RO177 ∪ RO227 ∪ RO315` (Northern Ireland's register), the set
`ods find --gp` expands to.

## Filtering on a role without duplicating rows

`roles.parquet` is one row per organisation *per role holding*, and a handful
of organisations hold the same role twice over — the role lapsed and was
re-added, so both an `inactive` and an `active` row exist for it. Join and
filter on `role_code` alone and you'll count some organisations twice:

```sql
-- wrong: pulls in the inactive holding alongside the active one
SELECT count(*) AS rows, count(DISTINCT o.ods_code) AS orgs
FROM 'ods_data/current/orgs.parquet' o
JOIN 'ods_data/current/roles.parquet' r ON o.ods_code = r.ods_code
WHERE o.status = 'active' AND r.role_code = 'RO270';
-- 16773 rows | 16204 orgs

-- right: role_codes is already the current, deduplicated answer
SELECT count(*) FROM 'ods_data/current/orgs.parquet'
WHERE status = 'active' AND list_contains(role_codes, 'RO270');
-- 16027
```

Add `role_status = 'active'` to the join and the count lands on 16027 too — but
you don't need the join at all. `role_codes` (and `role_names` beside it)
already carries what an organisation currently is. `roles.parquet` is for the
history of a holding, not the current fact.

## Where things are

You can group things by `town`.

```sql
-- Most GPs per town
SELECT town, count(*) AS GPs
FROM 'ods_data/current/orgs.parquet'
WHERE status = 'active' AND list_contains(role_names, 'GP Practice')
GROUP BY 1 ORDER BY 2 DESC LIMIT 5;
```
```
┌────────────┬───────┐
│    town    │  GPs  │
│  varchar   │ int64 │
├────────────┼───────┤
│ LONDON     │   685 │
│ GLASGOW    │   214 │
│ BIRMINGHAM │   173 │
│ MANCHESTER │   151 │
│ LIVERPOOL  │   124 │
└────────────┴───────┘
```

`postcode` is always space separated, so a prefix match is safe.

```sql
-- find active GPs in SW9
SELECT ods_code, name, role_names, postcode
FROM 'ods_data/current/orgs.parquet'
WHERE status = 'active' AND postcode LIKE 'SW9 %' AND list_contains(role_names, 'GP Practice');
```

## Who's in charge of it

There's no derived hierarchy column for this — join `relationships.parquet` on
`RE4 IS COMMISSIONED BY` to find who commissions something:

```sql
SELECT icb.name AS icb_name, count(*) AS practices
FROM 'ods_data/current/relationships.parquet' rel
JOIN 'ods_data/current/orgs.parquet' gp ON rel.source_code = gp.ods_code
JOIN 'ods_data/current/orgs.parquet' icb ON rel.target_code = icb.ods_code
WHERE gp.status = 'active' AND rel.rel_code = 'RE4' AND rel.rel_status = 'active'
  AND list_contains(gp.role_names, 'GP Practice')
GROUP BY 1 ORDER BY 2 DESC LIMIT 5;
```

| icb_name | practices |
| :--- | ---: |
| NHS WEST AND NORTH LONDON ICB - W2U3Z | 340 |
| NHS NORTH EAST LONDON ICB - A3A8R | 264 |
| NHS SOUTH EAST LONDON ICB - 72Q | 197 |
| NHS WEST AND NORTH LONDON ICB - 93C | 180 |
| NHS BIRMINGHAM AND SOLIHULL ICB - 15E | 177 |

ICBs commission through locality-level sub-codes (the `- W2U3Z` suffix), so the
same ICB name can show up more than once here — `RE4`'s target is the exact
commissioning body ODS recorded, not a rolled-up parent.

Same shape, counting buildings instead of practices — swap `RE4 IS COMMISSIONED
BY` for `RE6 IS OPERATED BY`:

```sql
SELECT trust.name AS trust_name, count(*) AS sites
FROM 'ods_data/current/relationships.parquet' rel
JOIN 'ods_data/current/orgs.parquet' s ON rel.source_code = s.ods_code
JOIN 'ods_data/current/orgs.parquet' trust ON rel.target_code = trust.ods_code
WHERE s.status = 'active' AND rel.rel_code = 'RE6' AND rel.rel_status = 'active'
  AND s.record_class = 'site'
GROUP BY 1 ORDER BY 2 DESC LIMIT 3;
-- TEES, ESK AND WEAR VALLEYS NHS FOUNDATION TRUST      1126
-- MIDLANDS PARTNERSHIP UNIVERSITY NHS FOUNDATION TRUST 938
-- SPECSAVERS HEARCARE GROUP LTD                        915
```

`RE6`'s target isn't always an NHS trust — any operator counts, private
providers included. Same lesson as [ODS has no hospital
concept](#ods-has-no-hospital-concept): the register doesn't have a role for
"is a hospital", and it doesn't have one for "is an NHS trust" either. Leave
`status = 'active'` off the target side (`trust` above) unless you mean it: a
trust that has since closed is still the operator ODS recorded — see
[the trap below](#traps) for how filtering a join's far side silently drops rows.


## Differences between `org` and `site` (IS OPERATED BY)

- **Almost 100% of sites (63,048 out of 63,066)** have an `IS OPERATED BY` relationship pointing to their parent organisation (e.g. an NHS Trust or Foundation Trust).
- While some `org` records also have this (e.g., branches or managed services, ~41%), it is effectively mandatory/universal for `site`.

Sites are physical operational locations, not legal contracting entities. As a result, **no site has any of the legal, commissioning, or governance relationships**:

| Relationship | Sites as Source | Orgs as Source | Meaning |
| :--- | :---: | :---: | :--- |
| **`IS COMMISSIONED BY`** | **0** | 114,414 | Sites cannot hold commissioning contracts with ICBs. |
| **`IS PARTNER TO`** | **0** | 7,602 | Partnerships are between legal bodies (PCNs, trusts). |
| **`IS DIRECTED BY`** | **0** | 4,048 | Governance directing bodies only apply to orgs. |
| **`IS CONSTITUENT OF`** | **0** | 1,945 | Membership (e.g. practices constituent of PCNs). |
| **`IS COVID NOMINATED PAYEE FOR`** | **0** | 897 | Financial payee nominations are legal orgs only. |

A `site` is **never the `target_code`** in any relationship (0 occurrences across all relationships). In ODS, relationships flow outward from a site to its operating org or geography (`Site -> IS OPERATED BY -> Org`), but other entities never link back to a site as their target.

| Relationship (`rel_name`) | Sites (`site`) | Orgs (`org`) |
| :--- | :---: | :---: |
| **`IS OPERATED BY`** | **63,102** (100% of sites) | 73,887 (41% of orgs) |
| **`IS LOCATED IN THE GEOGRAPHY OF`** | **45,382** (72% of sites) | 201,903 |
| **`IS NOMINATED PAYEE FOR`** | **47** (<0.1% legacy edge-cases) | 1,592 |
| **`IS A SUB-DIVISION OF`** | **18** | 127 |
| *All 5 other relationship types* | **0** | 128,806 |

Query to verify:

```sql
SELECT 
    r.rel_name,
    count(CASE WHEN o.record_class = 'site' THEN 1 END) AS site_source_count,
    count(CASE WHEN o.record_class = 'org' THEN 1 END) AS org_source_count
FROM 'ods_data/releases/2026-08-28/relationships.parquet' r
JOIN 'ods_data/releases/2026-08-28/orgs.parquet' o 
    ON r.source_code = o.ods_code
WHERE o.status = 'active'
GROUP BY r.rel_name
ORDER BY site_source_count DESC;
```

## What did this become

```sql
SELECT ods_code, name FROM 'ods_data/current/orgs.parquet'
WHERE status = 'active' AND list_contains(predecessor_codes, '0AF');
-- YDDTR | NHS GREATER MANCHESTER SHARED SERVICES
```

`predecessor_codes` holds the whole chain, not just the step before it, so you
never have to walk it yourself. `0AF` became `0CE`, which became `0CY`, which
became `YDDTR` — and all three are listed on `YDDTR`, along with the other two
organisations that merged in on the way.

With `status = 'active'` you get the live descendants... drop it to include the
ones that have since closed themselves (`0CE` and `0CY`, here).
See [parquet.md] for why there's no single "terminal successor".

## Use the operational dates

ODS carries two date families and you want `operational_*` nearly always. They're
the ones that are actually there:

- `operational_start` — every row, all 370,917 of them
- `operational_end` — every inactive row, plus 623 active ones with a closure
  already scheduled _(the furthest out is 2028-08-31)_
- `legal_start` — 19% of rows. 5.8% of the active ones.
- `legal_end` — 17% of rows

Use `status = 'active'` to find what's open today, not `operational_end IS NULL`,
which drops those 623.

**For experts only.** Legal dates are reliable for organisations created by
statute, and useless for everyone else, because a GP partnership or a corner shop
pharmacy has an opening day but no Act of Parliament. It shows in the fill rate:

| role | rows | has `legal_start` |
| :--- | ---: | ---: |
| CCG | 344 | 96.8% |
| Local Authority - Legacy | 420 | 89.3% |
| Primary Care Trust Site | 15,901 | 88.6% |
| Primary Care Trust | 398 | 82.9% |
| Local Authority Site - Legacy | 304 | 74.3% |
| General Dental Practice | 13,888 | 15.8% |
| GP Practice | 10,112 | 10.1% |

Even the statutory bodies aren't at 100%, so check the fill rate for your subset
before you rely on it rather than trusting the rule.

The failure is silent, which is what makes it worth the warning. `legal_start <=
DATE '...'` is false for NULL, so unpopulated rows just vanish from the result:

```sql
-- what existed on 2019-03-31, asked both ways, same file
SELECT
 (SELECT count(*) FROM 'ods_data/current/orgs.parquet'
   WHERE operational_start <= DATE '2019-03-31'
     AND (operational_end IS NULL OR operational_end > DATE '2019-03-31')) AS via_operational,
 (SELECT count(*) FROM 'ods_data/current/orgs.parquet'
   WHERE legal_start <= DATE '2019-03-31'
     AND (legal_end IS NULL OR legal_end > DATE '2019-03-31')) AS via_legal;
-- 204,991 | 9,088
```

Where both starts are present they disagree on 9,996 rows, and `legal_start` is
never the earlier of the two. Statutory recognition trails operation. It never
leads it.

## Names are not identifiers

```sql
SELECT name, count(*) AS codes FROM 'ods_data/current/orgs.parquet'
WHERE status = 'active'
GROUP BY 1 HAVING count(*) > 1 ORDER BY 2 DESC LIMIT 3;
-- DENTAL SURGERY 1679 · BOOTS 1411 · WELL 547
```

Match on name and you've matched 1,679 dental surgeries to each other. Join on
`ods_code`, always.

## Other things to check before you trust a result

**Ignore `operational_start` dates of `1900-01-01`.** 12,471 rows have one,
mostly schools. It's filler, not a claim about 1900, and it'll skew anything that
takes a `min()` or measures an age.

```sql
SELECT count(*) FROM 'ods_data/current/orgs.parquet'
WHERE operational_start = DATE '1900-01-01';
```

**Use a left join or an anti-join with `relationships.parquet`.** 610 active
entities have no relationships at all, so an inner join drops them without
saying so:

```sql
WITH linked AS (
  SELECT source_code AS code FROM 'ods_data/current/relationships.parquet'
  UNION
  SELECT target_code       FROM 'ods_data/current/relationships.parquet'
)
SELECT count(*) FROM 'ods_data/current/orgs.parquet'
WHERE status = 'active' AND ods_code NOT IN (SELECT code FROM linked);
-- 610
```

## Across releases

Every table carries `trud_release_date`, so a directory of pinned releases is a
time series and one glob reads the lot.

```sql
SELECT trud_release_date, count(*) AS n_rows,
       count(*) FILTER (WHERE status = 'active') AS n_active
FROM read_parquet('ods_data/releases/*/orgs.parquet')
GROUP BY 1 ORDER BY 1;
```

| trud_release_date | n_rows | n_active |
| :--- | ---: | ---: |
| 2026-05-29 | 368,495 | 216,564 |
| 2026-06-26 | 369,400 | 216,417 |
| 2026-07-31 | 370,257 | 216,886 |
| 2026-08-28 | 370,917 | 217,059 |

Nearly everything below is the same idiom with a different column plugged in:

```sql
lag(anything) OVER (PARTITION BY ods_code ORDER BY trud_release_date)
```

Four releases scan in under 0.1s, so this stays snappy well past a decade of
monthly snapshots.

## What opened and closed

```sql
WITH s AS (
  SELECT ods_code, name, trud_release_date, status,
         lag(status) OVER (PARTITION BY ods_code ORDER BY trud_release_date) AS prev_status
  FROM read_parquet('ods_data/releases/*/orgs.parquet')
)
SELECT trud_release_date, prev_status, status, count(*) AS n
FROM s WHERE prev_status IS NOT NULL AND prev_status <> status
GROUP BY ALL ORDER BY 1, 2;
```

| trud_release_date | was | now | n |
| :--- | :--- | :--- | ---: |
| 2026-06-26 | active | inactive | 1068 |
| 2026-06-26 | inactive | active | 18 |
| 2026-07-31 | active | inactive | 618 |
| 2026-07-31 | inactive | active | 44 |
| 2026-08-28 | active | inactive | 610 |
| 2026-08-28 | inactive | active | 123 |

The reopenings are the half nobody expects. 185 organisations came back from the
dead over three months, and no single release records that it ever happened.

## Deletions happen, so keep your snapshots

186 codes that were in the May release are gone from August. Not closed — gone.
Every one of them an inactive organisation when last seen, mostly schools.

```sql
WITH r AS (SELECT ods_code, trud_release_date FROM read_parquet('ods_data/releases/*/orgs.parquet'))
SELECT count(*) FROM (
  SELECT ods_code FROM r GROUP BY 1 HAVING max(trud_release_date) < DATE '2026-08-28'
);
-- 186
```

ODS deletes rows as well as retiring them, and 2,608 new codes turned up over the
same window. Between the May and August releases 165 codes disappeared from the
live file, and none moved into the archive: the archive holds what NHS England
moves out at the cut-off, not what it deletes. Across both files 186 disappeared
and 2,608 appeared. So "every entity ODS has ever known" is the union of the
releases you kept, not the newest `orgs.parquet`. Keep your snapshots.

## Name and address history

One release carries one current name per entity. Two releases carry the change.

```sql
WITH s AS (
  SELECT ods_code, name, trud_release_date,
         lag(name) OVER (PARTITION BY ods_code ORDER BY trud_release_date) AS was
  FROM read_parquet('ods_data/releases/*/orgs.parquet')
)
SELECT ods_code, was, name AS now, trud_release_date
FROM s WHERE was IS NOT NULL AND name <> was ORDER BY ods_code;
```

| ods_code | was | now |
| :--- | :--- | :--- |
| 8HR16 | BANTHAM TECHNOLOGIES LIMITED | INKWRX LIMITED |
| 8HQ46 | CSAM (UK) LTD | OMDA HEALTH ANALYTICS LIMITED |
| 8JH56 | AGE CONCERN BIRMINGHAM | AGE CONNECT WEST MIDLANDS |

330 renames in June, 320 in July, 257 in August, and 439, 407 then 316 postcode changes. Swap `name`
for `address`, `town` or `uprn` to watch things move house.

## Which release should you use

- **The latest one** for questions about history. ODS carries on recording
  closures and successions after the event, so the newest release knows the most
  about the past.
- **The release from the time** if you need what ODS knew then. Reproducing a
  published figure, or checking what a service saw when it made a decision.
- **The whole archive** for names, addresses, and entities that have since been
  deleted from the register. Those only exist in the release that carried them.

What you shouldn't do is rebuild an old month by date-filtering a newer release.
Ask what was active on `2026-05-29` both ways and the counts land within 6 of
each other (216,564 and 216,570), so it looks like it worked. They're not the same organisations
though:

```sql
WITH may AS (SELECT ods_code FROM 'ods_data/releases/2026-05-29/orgs.parquet'
              WHERE status = 'active'),
     jul AS (SELECT ods_code FROM 'ods_data/releases/2026-07-31/orgs.parquet'
              WHERE operational_start <= DATE '2026-05-29'
                AND (operational_end IS NULL OR operational_end > DATE '2026-05-29'))
SELECT (SELECT count(*) FROM (SELECT * FROM may EXCEPT SELECT * FROM jul)) AS in_may_only,
       (SELECT count(*) FROM (SELECT * FROM jul EXCEPT SELECT * FROM may)) AS in_rebuild_only;
-- 389 | 395
```

784 differences at two months' distance, netting out to 6. It's mostly ordinary
churn — things opening and closing — rather than ODS rewriting the past.
_(Records that were already closed rarely change: 1 of them in June and 7 in
July.)_

## Relationship churn over time

Care is needed when aggregating over relationships over time.
Commissioning links (`RE4`) and operational links (`RE6`) change across monthly releases.

```sql
WITH rel_history AS (
  SELECT source_code, rel_code, target_code, trud_release_date,
         lag(target_code) OVER (PARTITION BY source_code, rel_code ORDER BY trud_release_date) AS prev_target
  FROM read_parquet('ods_data/releases/*/relationships.parquet')
  WHERE rel_status = 'active'
)
SELECT trud_release_date,
  count(*) FILTER (WHERE rel_code = 'RE4' AND target_code IS DISTINCT FROM prev_target) AS commissioner_changed,
  count(*) FILTER (WHERE rel_code = 'RE6' AND target_code IS DISTINCT FROM prev_target) AS operated_by_changed
FROM rel_history WHERE trud_release_date > DATE '2026-05-29'
GROUP BY 1 ORDER BY 1;
```

| trud_release_date | commissioner_changed | operated_by_changed |
| :--- | ---: | ---: |
| 2026-06-26 | 140 | 648 |
| 2026-07-31 | 158 | 776 |
| 2026-08-28 | 99 | 488 |

## Is `last_changed` honest?

ODS publishes a `last_changed` date on every record. With an archive you can
check whether it's telling the truth, instead of hoping.

```sql
WITH s AS (
  SELECT ods_code, trud_release_date, last_changed, name, postcode, status, primary_role_code,
         lag(last_changed)      OVER w AS p_changed,  lag(name)     OVER w AS p_name,
         lag(postcode)          OVER w AS p_postcode, lag(status)   OVER w AS p_status,
         lag(primary_role_code) OVER w AS p_role
  FROM read_parquet('ods_data/releases/*/orgs.parquet')
  WINDOW w AS (PARTITION BY ods_code ORDER BY trud_release_date)
), c AS (
  SELECT *,
    (name IS DISTINCT FROM p_name OR postcode IS DISTINCT FROM p_postcode
     OR status IS DISTINCT FROM p_status OR primary_role_code IS DISTINCT FROM p_role) AS edited,
    (last_changed IS DISTINCT FROM p_changed) AS stamped
  FROM s WHERE p_changed IS NOT NULL
)
SELECT trud_release_date,
  count(*) FILTER (WHERE edited AND stamped)     AS edited_and_stamped,
  count(*) FILTER (WHERE edited AND NOT stamped) AS edited_not_stamped,
  count(*) FILTER (WHERE NOT edited AND stamped) AS stamped_no_visible_edit
FROM c GROUP BY 1 ORDER BY 1;
```

| trud_release_date | edited & stamped | edited, not stamped | stamped, no visible edit |
| :--- | ---: | ---: | ---: |
| 2026-06-26 | 1797 | 0 | 1093 |
| 2026-07-31 | 1335 | 0 | 3459 |
| 2026-08-28 | 1267 | 0 | 758 |

Not one unstamped edit in any month, across name, postcode, status and primary
role. That's a good result for ODS. _(It only covers those four fields over three
months, mind. The right hand column is edits to roles and relationships, which
don't show up as `orgs` columns.)_

## Successions turn up late

```sql
WITH s AS (
  SELECT ods_code, trud_release_date, successor_codes,
         lag(successor_codes) OVER (PARTITION BY ods_code ORDER BY trud_release_date) AS was
  FROM read_parquet('ods_data/releases/*/orgs.parquet')
)
SELECT trud_release_date, count(*) AS gained_successors
FROM s WHERE was IS NOT NULL AND len(successor_codes) > len(was)
GROUP BY 1 ORDER BY 1;
-- 2026-07-31 | 6
```

The paperwork lands after the merger, so a mapping built from one release can be
incomplete for anything recent.

## Traps

**Join `relationships.parquet` through `orgs` and mean it.** A relationship can point at
an organisation that's since closed, even while the relationship itself is current.
`orgs.parquet` holds closed organisations too, so a plain join keeps every row. Add
`o.status = 'active'` to the join and you silently lose a third of the table:

```sql
SELECT count(*) FROM 'ods_data/current/relationships.parquet' rel
JOIN 'ods_data/current/orgs.parquet' o ON rel.source_code = o.ods_code AND o.status = 'active';
-- 514964

SELECT count(*) FROM 'ods_data/current/relationships.parquet' rel
JOIN 'ods_data/current/orgs.parquet' o ON rel.source_code = o.ods_code;
-- 769527
```

This is the worst trap here. It looks like it worked either way — both queries
return a large, plausible, non-empty result. Nothing tells you a third of the
graph went missing. Filter the organisation you're asking about in `WHERE`,
and leave the far side of the join alone.

**Filter `rel_status = 'active'` on `relationships.parquet`, or count the past
as the present.** More than half the table is history:

```sql
SELECT rel_status, count(*) FROM 'ods_data/current/relationships.parquet' GROUP BY 1;
-- inactive | 466675
-- active   | 302852
```

**Group by `trud_release_date` for release identity.** It aligns with the directory
names and TRUD release distributions (e.g. `2026-05-29`, `2026-06-26`, `2026-07-31`, `2026-08-28`).
It's TRUD's release date, recorded in every Parquet row and in each file's embedded
provenance (`sources[0].version`), not the XML manifest's own internal publication date,
which `ods` doesn't capture.

**Add `union_by_name = true` when your releases span a schema change.** Missing
columns read as NULL instead of failing the query. Pre-1.0 that's every schema
change, so just always.

**`filename = true`** adds the source path to every row, for when you need to
trace a result back to the release it came out of.

**Globs are a local thing.** DuckDB expands them over local paths and object
stores, not plain HTTPS, so querying published releases remotely means listing
the URLs out:

```sql
SELECT * FROM read_parquet([
  'https://…/2026-06-26/orgs.parquet',
  'https://…/2026-07-31/orgs.parquet'
]);
```

**`WHERE operational_end IS NULL` does not equal `status = 'active'`.** Upstream TRUD ODS contains 623 organisations that are `Active` while carrying an `Operational` end date. Anyone filtering `WHERE operational_end IS NULL` to mean "currently open" silently drops those 623 active organisations. Always use `WHERE status = 'active'` to filter currently open organisations.

**Active relationships on inactive organisations.** Upstream TRUD ODS contains 3 relationships marked `Active` associated with `Inactive` organisations.

[parquet.md]: ./parquet.md
