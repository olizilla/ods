# Queries

Some interesting queries with real output. The schema and the reasoning behind it is in [parquet.md].

You can point `duckdb` at the parquet files and go. Local-first! All examples here show the output given from the `2026-07-31` release.

```console
$ duckdb -c "SELECT ods_code, name, role_names FROM 'ods_data/current/orgs.parquet' WHERE town = 'SEDBERGH'"
┌───────────┬──────────────────────────────────────────┬────────────────────────────────────────┐
│ ods_code  │                   name                   │               role_names               │
├───────────┼──────────────────────────────────────────┼────────────────────────────────────────┤
│ A82608001 │ DR LUMB W & PARTNER                      │ [Branch Surgery]                       │
│ VN6C2     │ PRIVATE PERSONAL ASSISTANCE LIMITED      │ [Domiciliary Care]                     │
│ A82608    │ SEDBERGH MEDICAL PRACTICE                │ [GP Practice, Prescribing Cost Centre] │
│ V25604    │ MAIN STREET DENTAL SURGERY               │ [General Dental Practice]              │
│ RNN88     │ SEDBERGH HEALTH CENTRE                   │ [NHS Trust Site]                       │
│ RW5OX     │ SEDBURGH MEDICAL CENTRE                  │ [NHS Trust Site]                       │
│ RX796     │ SEDBURGH AMBULANCE STATION               │ [NHS Trust Site]                       │
│ 8GJ58     │ PARKER M JUNE (ACUPUNCURIST)             │ [Non-NHS Organisation]                 │
│ FLG02     │ ALLIED PHARMACY SEDBERGH                 │ [Pharmacy]                             │
│ D2E8H     │ AP SD THIRTEEN LIMITED                   │ [Pharmacy Headquarter]                 │
│ EE137269  │ SETTLEBECK SCHOOL                        │ [School]                               │
│ EE112233  │ SEDBERGH PRIMARY SCHOOL                  │ [School]                               │
│ EE112331  │ DENT COFE VOLUNTARY AIDED PRIMARY SCHOOL │ [School]                               │
│ EE112451  │ SEDBERGH SCHOOL                          │ [School]                               │
└───────────┴──────────────────────────────────────────┴────────────────────────────────────────┘
```

The numbers here come from the release `current` points at, `2026-07-31`. Yours
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
- **You can filter on `country` if you need to.** ODS covers the UK and dependencies, containing six distinct country values: `ENGLAND` (209,347), `WALES` (5,753), `SCOTLAND` (1,178), `NORTHERN IRELAND` (354), `ISLE OF MAN` (200), and `CHANNEL ISLANDS` (54).
- **Expect a list of successors, not one.** ODS code `001` has five. Following just
  the first is a dead end, and unresolved mergers are a leading source of error in
  longitudinal analysis of NHS data.
- **[Prefer `operational` dates](#use-the-operational-dates)** to find when a thing was active. `legal_*` dates can be
  different to "when a site was operational" and are often not available.
- **[Join on `ods_code`](#names-are-not-identifiers).** Names aren't unique identifiers. 1,680 entities are called `DENTAL SURGERY`.
- **[Use the latest release for history](#which-release-should-you-use)**, and the
  release from the time if you need what ODS knew then. Entities get registered,
  closed, reopened and sometimes deleted outright between releases.
- **[Compare releases to see a name or address change](#name-and-address-history).**
  Each release only carries the current name for an entity.

## What kind of thing is it

```sql
SELECT unnest(role_names) AS role, count(*) AS n
FROM 'ods_data/current/orgs.parquet'
WHERE record_class = 'org' GROUP BY 1 ORDER BY n DESC LIMIT 8;
```
```
┌─────────────────────────┬───────┐
│          role           │   n   │
│         varchar         │ int64 │
├─────────────────────────┼───────┤
│ School                  │ 25187 │
│ Social Care Provider    │ 18681 │
│ Non-NHS Organisation    │ 17142 │
│ Care Home               │ 16024 │
│ Domiciliary Care        │ 15670 │
│ Prescribing Cost Centre │ 12841 │
│ Pharmacy                │ 11177 │
│ General Dental Practice │  9789 │
└─────────────────────────┴───────┘
```

Because `role_names` is positionally aligned with `role_codes`, you can derive
the primary role name without a join:

```sql
SELECT name, role_names[list_position(role_codes, primary_role_code)] AS primary_role_name
FROM 'ods_data/current/orgs.parquet'
LIMIT 5;
```

Most of ODS isn't what you'd picture as the NHS. Schools and care homes
outnumber NHS sites, because ODS registers everyone who exchanges data with the
NHS, not just the bits the NHS owns.

Filter on `role_names` or `role_codes`. >92k active entities have more than one role:

```sql
SELECT len(role_codes) AS roles, count(*) AS entities
FROM 'ods_data/current/orgs.parquet' GROUP BY 1 ORDER BY 1;
```
```
┌───────┬──────────┐
│ roles │ entities │
│ int64 │  int64   │
├───────┼──────────┤
│     1 │   123992 │
│     2 │    92297 │
│     3 │      594 │
│     4 │        3 │
└───────┴──────────┘
```

## ODS has no hospital concept

A hospital building, its A&E and its fertility unit are all the same role,
`RO198 NHS Trust Site` — there's no `Hospital` role to filter on.

```sql
SELECT count(*) AS all_sites,
       count(*) FILTER (WHERE name LIKE '%HOSPITAL%') AS named_hospital
FROM 'ods_data/current/orgs.parquet' WHERE list_contains(role_codes, 'RO198');
-- 38254 | 3681
```

Only 1 in 10 sites has "hospital" in its name. To find a hospital's whole
estate, find its trust and search that trust's sites via `relationships.parquet`:

```sql
SELECT s.ods_code, s.name
FROM 'ods_data/current/relationships.parquet' r
JOIN 'ods_data/current/orgs.parquet' s ON r.source_code = s.ods_code
WHERE r.target_code = 'RRV' AND r.rel_code = 'RE6' AND r.rel_status = 'active';
```

## RO177 is England's prescribing register, not the UK's

```sql
SELECT count(*) AS scottish_gp_practices,
       count(*) FILTER (WHERE list_contains(role_codes, 'RO177')) AS also_ro177
FROM 'ods_data/current/orgs.parquet' WHERE list_contains(role_codes, 'RO227');
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
WHERE r.role_code = 'RO270';
-- 16726 rows | 16157 orgs

-- right: role_codes is already the current, deduplicated answer
SELECT count(*) FROM 'ods_data/current/orgs.parquet'
WHERE list_contains(role_codes, 'RO270');
-- 15977
```

Add `role_status = 'active'` to the join and the count lands on 15977 too — but
you don't need the join at all. `role_codes` (and `role_names` beside it)
already carries what an organisation currently is. `roles.parquet` is for the
history of a holding, not the current fact.

## Where things are

You can group things by `town`.

```sql
-- Most GPs per town
SELECT town, count(*) AS GPs
FROM 'ods_data/current/orgs.parquet'
WHERE list_contains(role_names, 'GP Practice')
GROUP BY 1 ORDER BY 2 DESC LIMIT 5;
```
```
┌────────────┬───────┐
│    town    │  GPs  │
│  varchar   │ int64 │
├────────────┼───────┤
│ LONDON     │   686 │
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
WHERE postcode LIKE 'SW9 %' AND list_contains(role_names, 'GP Practice');
```

## Who's in charge of it

The derived hierarchy columns save you a trip through `relationships.parquet`.

```sql
SELECT icb_name, count(*) AS practices
FROM 'ods_data/current/orgs.parquet'
WHERE list_contains(role_names, 'GP Practice') AND icb_name IS NOT NULL
GROUP BY 1 ORDER BY 2 DESC LIMIT 5;
```

| icb_name | practices |
| :--- | ---: |
| NHS WEST AND NORTH LONDON INTEGRATED CARE BOARD | 521 |
| NHS GREATER MANCHESTER INTEGRATED CARE BOARD | 409 |
| NHS NORTH EAST AND NORTH CUMBRIA INTEGRATED CARE BOARD | 339 |
| NHS CHESHIRE AND MERSEYSIDE INTEGRATED CARE BOARD | 338 |
| NHS CENTRAL EAST INTEGRATED CARE BOARD | 271 |

Same shape, counting buildings instead of practices:

```sql
SELECT trust_name, count(*) AS sites
FROM 'ods_data/current/orgs.parquet'
WHERE record_class = 'site' AND trust_name IS NOT NULL
GROUP BY 1 ORDER BY 2 DESC LIMIT 3;
-- TEES, ESK AND WEAR VALLEYS NHS FOUNDATION TRUST             1121
-- MIDLANDS PARTNERSHIP UNIVERSITY NHS FOUNDATION TRUST        930
-- HAMPSHIRE AND ISLE OF WIGHT HEALTHCARE NHS FOUNDATION TRUST 812
```

A NULL means ODS records no such relationship, not that we failed to resolve it.
Go to `relationships.parquet` when you need the relationship's own dates or
status.

## What did this become

```sql
SELECT ods_code, name FROM 'ods_data/current/orgs.parquet'
WHERE list_contains(predecessor_codes, '0AF');
-- YDDTR | NHS GREATER MANCHESTER SHARED SERVICES
```

`predecessor_codes` holds the whole chain, not just the step before it, so you
never have to walk it yourself. `0AF` became `0CE`, which became `0CY`, which
became `YDDTR` — and all three are listed on `YDDTR`, along with the other two
organisations that merged in on the way.

Because `orgs.parquet` is active-only you get the live descendants... run it
against `orgs_all.parquet` to include the ones that have since closed themselves.
See [parquet.md] for why there's no single "terminal successor".

## Use the operational dates

ODS carries two date families and you want `operational_*` nearly always. They're
the ones that are actually there:

- `operational_start` — every row, all 305,541 of them
- `operational_end` — every inactive row, plus 603 active ones with a closure
  already scheduled _(the furthest out is 2028-08-31)_
- `legal_start` — 14% of rows. 5.8% of the active ones.
- `legal_end` — 12% of rows

Use `status = 'active'` to find what's open today, not `operational_end IS NULL`,
which drops those 603.

**For experts only.** Legal dates are reliable for organisations created by
statute, and useless for everyone else, because a GP partnership or a corner shop
pharmacy has an opening day but no Act of Parliament. It shows in the fill rate:

| role | rows | has `legal_start` |
| :--- | ---: | ---: |
| Local Authority Site - Legacy | 222 | 100% |
| CCG | 344 | 96.8% |
| Primary Care Trust Site | 14,693 | 93.8% |
| Local Authority - Legacy | 418 | 89.7% |
| Primary Care Trust | 395 | 83.5% |
| GP Practice | 9,372 | 6.4% |
| General Dental Practice | 12,179 | 4.2% |

Even the statutory bodies aren't at 100%, so check the fill rate for your subset
before you rely on it rather than trusting the rule.

The failure is silent, which is what makes it worth the warning. `legal_start <=
DATE '...'` is false for NULL, so unpopulated rows just vanish from the result:

```sql
-- what existed on 2019-03-31, asked both ways, same file
SELECT
 (SELECT count(*) FROM 'ods_data/current/orgs_all.parquet'
   WHERE operational_start <= DATE '2019-03-31'
     AND (operational_end IS NULL OR operational_end > DATE '2019-03-31')) AS via_operational,
 (SELECT count(*) FROM 'ods_data/current/orgs_all.parquet'
   WHERE legal_start <= DATE '2019-03-31'
     AND (legal_end IS NULL OR legal_end > DATE '2019-03-31')) AS via_legal;
-- 204,991 | 9,088
```

Where both starts are present they disagree on 9,750 rows, and `legal_start` is
never the earlier of the two. Statutory recognition trails operation. It never
leads it.

## Names are not identifiers

```sql
SELECT name, count(*) AS codes FROM 'ods_data/current/orgs.parquet'
GROUP BY 1 HAVING count(*) > 1 ORDER BY 2 DESC LIMIT 3;
-- DENTAL SURGERY 1680 · BOOTS 1412 · WELL 550
```

Match on name and you've matched 1,680 dental surgeries to each other. Join on
`ods_code`, always.

## Other things to check before you trust a result

**Ignore `operational_start` dates of `1900-01-01`.** 12,470 rows have one,
mostly schools. It's filler, not a claim about 1900, and it'll skew anything that
takes a `min()` or measures an age.

```sql
SELECT count(*) FROM 'ods_data/current/orgs_all.parquet'
WHERE operational_start = DATE '1900-01-01';
```

**Use a left join or an anti-join with `relationships.parquet`.** 636 active
entities have no relationships at all, so an inner join drops them without
saying so:

```sql
WITH linked AS (
  SELECT source_code AS code FROM 'ods_data/current/relationships.parquet'
  UNION
  SELECT target_code       FROM 'ods_data/current/relationships.parquet'
)
SELECT count(*) FROM 'ods_data/current/orgs.parquet'
WHERE ods_code NOT IN (SELECT code FROM linked);
```

## Across releases

Every table carries `trud_release_date`, so a directory of pinned releases is a
time series and one glob reads the lot.

```sql
SELECT trud_release_date, count(*) AS n_rows,
       count(*) FILTER (WHERE status = 'active') AS n_active
FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
GROUP BY 1 ORDER BY 1;
```

| trud_release_date | n_rows | n_active |
| :--- | ---: | ---: |
| 2026-05-29 | 303,756 | 216,564 |
| 2026-06-26 | 304,662 | 216,417 |
| 2026-07-31 | 305,541 | 216,886 |

Nearly everything below is the same idiom with a different column plugged in:

```sql
lag(anything) OVER (PARTITION BY ods_code ORDER BY trud_release_date)
```

Three releases scan in under 0.1s, so this stays snappy well past a decade of
monthly snapshots.

## What opened and closed

```sql
WITH s AS (
  SELECT ods_code, name, trud_release_date, status,
         lag(status) OVER (PARTITION BY ods_code ORDER BY trud_release_date) AS prev_status
  FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
)
SELECT trud_release_date, prev_status, status, count(*) AS n
FROM s WHERE prev_status IS NOT NULL AND prev_status <> status
GROUP BY ALL ORDER BY 1, 2;
```

| trud_release_date | was | now | n |
| :--- | :--- | :--- | ---: |
| 2026-06-26 | active | inactive | 1068 |
| 2026-06-26 | inactive | active | 17 |
| 2026-07-31 | active | inactive | 618 |
| 2026-07-31 | inactive | active | 43 |

The reopenings are the half nobody expects. 60 organisations came back from the
dead over two months, and no single release records that it ever happened.

## The archive is the only complete list

165 codes that were in the May release are gone from July. Not closed — gone.
Every one of them an inactive organisation when last seen, mostly schools.

```sql
WITH r AS (SELECT ods_code, trud_release_date FROM read_parquet('ods_data/releases/*/orgs_all.parquet'))
SELECT count(*) FROM (
  SELECT ods_code FROM r GROUP BY 1 HAVING max(trud_release_date) < DATE '2026-07-31'
);
-- 165
```

ODS deletes rows as well as retiring them, and 1,950 new codes turned up over the
same window. So "every entity ODS has ever known" is the union of the releases
you kept, not the newest `orgs_all.parquet`. Keep your snapshots.

## Name and address history

One release carries one current name per entity. Two releases carry the change.

```sql
WITH s AS (
  SELECT ods_code, name, trud_release_date,
         lag(name) OVER (PARTITION BY ods_code ORDER BY trud_release_date) AS was
  FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
)
SELECT ods_code, was, name AS now, trud_release_date
FROM s WHERE was IS NOT NULL AND name <> was ORDER BY ods_code;
```

| ods_code | was | now |
| :--- | :--- | :--- |
| 8HQ46 | CSAM (UK) LTD | OMDA HEALTH ANALYTICS LIMITED |
| 8JH56 | AGE CONCERN BIRMINGHAM | AGE CONNECT WEST MIDLANDS |
| 8HR16 | BANTHAM TECHNOLOGIES LIMITED | INKWRX LIMITED |

330 renames in June, 320 in July, and 439 then 407 postcode changes. Swap `name`
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
each other, so it looks like it worked. They're not the same organisations
though:

```sql
WITH may AS (SELECT ods_code FROM 'ods_data/releases/2026-05-29/orgs_all.parquet'
              WHERE status = 'active'),
     jul AS (SELECT ods_code FROM 'ods_data/releases/2026-07-31/orgs_all.parquet'
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
| 2026-06-26 | 253 | 406 |
| 2026-07-31 | 281 | 541 |

## Is `last_changed` honest?

ODS publishes a `last_changed` date on every record. With an archive you can
check whether it's telling the truth, instead of hoping.

```sql
WITH s AS (
  SELECT ods_code, trud_release_date, last_changed, name, postcode, status, primary_role_code,
         lag(last_changed)      OVER w AS p_changed,  lag(name)     OVER w AS p_name,
         lag(postcode)          OVER w AS p_postcode, lag(status)   OVER w AS p_status,
         lag(primary_role_code) OVER w AS p_role
  FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
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
| 2026-06-26 | 1796 | 0 | 1071 |
| 2026-07-31 | 1334 | 0 | 1730 |

Not one unstamped edit in either month, across name, postcode, status and primary
role. That's a good result for ODS. _(It only covers those four fields over two
months, mind. The right hand column is edits to roles and relationships, which
don't show up as `orgs` columns.)_

## Successions turn up late

```sql
WITH s AS (
  SELECT ods_code, trud_release_date, successor_codes,
         lag(successor_codes) OVER (PARTITION BY ods_code ORDER BY trud_release_date) AS was
  FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
)
SELECT trud_release_date, count(*) AS gained_successors
FROM s WHERE was IS NOT NULL AND len(successor_codes) > len(was)
GROUP BY 1 ORDER BY 1;
-- 2026-07-31 | 3
```

The paperwork lands after the merger, so a mapping built from one release can be
incomplete for anything recent.

## Traps

**`releases/*/orgs*.parquet` counts everything twice.** The glob matches
`orgs.parquet` _and_ `orgs_all.parquet` in every directory: 1,563,826 rows where
there are really 649,867. `orgs` is a subset of `orgs_all` by design. Name the
file you actually want.

**Join `relationships.parquet` through `orgs_all`, not `orgs`, unless you mean
to drop inactive endpoints.** A relationship can point at an organisation that's
since closed, even while the relationship itself is current. Joining through
`orgs` (active-only) silently loses 22% of the table:

```sql
SELECT count(*) FROM 'ods_data/current/relationships.parquet' rel
JOIN 'ods_data/current/orgs.parquet' o ON rel.source_code = o.ods_code;
-- 515049

SELECT count(*) FROM 'ods_data/current/relationships.parquet' rel
JOIN 'ods_data/current/orgs_all.parquet' o ON rel.source_code = o.ods_code;
-- 662558
```

This is the worst trap here. It looks like it worked either way — both queries
return a large, plausible, non-empty result. Nothing tells you a fifth of the
graph went missing.

**Filter `rel_status = 'active'` on `relationships.parquet`, or count the past
as the present.** More than half the table is history:

```sql
SELECT rel_status, count(*) FROM 'ods_data/current/relationships.parquet' GROUP BY 1;
-- inactive | 359912
-- active   | 302646
```

**Group by `trud_release_date` for release identity.** It aligns with the directory
names and TRUD release distributions (e.g. `2026-05-29`, `2026-06-26`, `2026-07-31`).
The internal XML publication date is preserved in `_provenance.json`.

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
  'https://…/2026-06-26/orgs_all.parquet',
  'https://…/2026-07-31/orgs_all.parquet'
]);
```

**`WHERE operational_end IS NULL` does not equal `status = 'active'`.** Upstream TRUD ODS contains 603 organisations that are `Active` while carrying an `Operational` end date. Anyone filtering `WHERE operational_end IS NULL` to mean "currently open" silently drops those 603 active organisations. Always use `WHERE status = 'active'` to filter currently open organisations.

**Active relationships on inactive organisations.** Upstream TRUD ODS contains 3 relationships marked `Active` associated with `Inactive` organisations.

[parquet.md]: ./parquet.md
