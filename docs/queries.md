# Queries

Worked examples with real output, and the sharp edges in the data that'll bite
you. Schemas and the reasoning behind them are in [parquet.md].

No setup, no extensions, no network. Point `duckdb` at a file and go.

```console
$ duckdb -c "SELECT ods_code, name, category FROM 'ods_data/current/orgs.parquet' WHERE town = 'SEDBERGH' ORDER BY category"
┌───────────┬──────────────────────────────────────────┬─────────────────────────┐
│ ods_code  │                   name                   │        category         │
├───────────┼──────────────────────────────────────────┼─────────────────────────┤
│ A82608001 │ DR LUMB W & PARTNER                      │ Branch Surgery          │
│ VN6C2     │ PRIVATE PERSONAL ASSISTANCE LIMITED      │ Domiciliary Care        │
│ A82608    │ SEDBERGH MEDICAL PRACTICE                │ GP Practice             │
│ V25604    │ MAIN STREET DENTAL SURGERY               │ General Dental Practice │
│ RNN88     │ SEDBERGH HEALTH CENTRE                   │ NHS Trust Site          │
│ RW5OX     │ SEDBURGH MEDICAL CENTRE                  │ NHS Trust Site          │
│ RX796     │ SEDBURGH AMBULANCE STATION               │ NHS Trust Site          │
│ 8GJ58     │ PARKER M JUNE (ACUPUNCURIST)             │ Non-NHS Organisation    │
│ FLG02     │ ALLIED PHARMACY SEDBERGH                 │ Pharmacy                │
│ D2E8H     │ AP SD THIRTEEN LIMITED                   │ Pharmacy Headquarter    │
│ EE137269  │ SETTLEBECK SCHOOL                        │ School                  │
│ EE112233  │ SEDBERGH PRIMARY SCHOOL                  │ School                  │
│ EE112331  │ DENT COFE VOLUNTARY AIDED PRIMARY SCHOOL │ School                  │
│ EE112451  │ SEDBERGH SCHOOL                          │ School                  │
└───────────┴──────────────────────────────────────────┴─────────────────────────┘
```

_(`SEDBURGH MEDICAL CENTRE` is misspelled in the source data, not by us.)_

The numbers here come from the `2026-07-31` release. Yours will differ if you're
pinned elsewhere... `ods pull --list` tells you what you've got. Single-release
queries come first, then the ones that need an archive of releases.

## The source data has issues

Worth knowing before you trust a query.

- **The primary role describes the register, not the organisation.** GP practices
  are `RO177 Prescribing Cost Centre`. Use `role_codes` or `category`.
- **Successions are many-to-many.** ODS code `001` has five successors. Taking the
  first one is wrong, and unresolved mergers are a leading source of error in
  longitudinal analysis of NHS data.
- **`legal_*` is mostly empty.** [Use the operational dates](#use-the-operational-dates).
- **Names aren't identifiers.** 1,680 entities are called `DENTAL SURGERY`.
  [Join on `ods_code`](#names-are-not-identifiers).
- **A snapshot taken today won't reproduce one taken in 2019.** Entities get
  registered, closed, reopened and sometimes deleted outright between releases.
  [Use the release from the date you care about](#why-a-pinned-release-beats-a-date-filter).
- **Name and address history isn't in any single release.** Each release carries
  one current name per entity, so a change is only visible
  [across releases](#name-and-address-history).

## What kind of thing is it

```sql
SELECT category, count(*) AS n
FROM 'ods_data/current/orgs.parquet'
GROUP BY 1 ORDER BY n DESC LIMIT 8;
```

| category | n |
| :--- | ---: |
| NHS Trust Site | 38,254 |
| School | 25,187 |
| Social Care Provider | 18,681 |
| Non-NHS Organisation | 17,142 |
| Independent Sector Healthcare Provider Site | 16,936 |
| Care Home | 16,024 |
| Domiciliary Care | 15,670 |
| Pharmacy | 11,177 |

Most of ODS isn't what you'd picture as the NHS. Schools and care homes
outnumber NHS sites, because ODS registers everyone who exchanges data with the
NHS, not just the bits the NHS owns.

For anything finer than `category`, filter on `role_codes` and join
`roles.parquet` for the label. Roles aren't a partition though — 92,894 active
entities hold more than one:

```sql
SELECT len(role_codes) AS n_roles, count(*) AS orgs
FROM 'ods_data/current/orgs.parquet' GROUP BY 1 ORDER BY 1;
-- 1 → 123,992 · 2 → 92,297 · 3 → 594 · 4 → 3
```

## Where is it

```sql
-- pharmacies by town
SELECT town, count(*) AS pharmacies
FROM 'ods_data/current/orgs.parquet'
WHERE category = 'Pharmacy' AND town IS NOT NULL
GROUP BY 1 ORDER BY 2 DESC LIMIT 5;
-- LONDON 1075 · BIRMINGHAM 266 · MANCHESTER 229 · LIVERPOOL 161 · NOTTINGHAM 151

-- everything in an outward postcode. `postcode` is always space separated,
-- so a prefix match is safe
SELECT ods_code, name, category, postcode
FROM 'ods_data/current/orgs.parquet'
WHERE postcode LIKE 'LA10 %' ORDER BY category;
```

## Who's in charge of it

The derived hierarchy columns save you a trip through `relationships.parquet`.

```sql
SELECT icb_name, count(*) AS practices
FROM 'ods_data/current/orgs.parquet'
WHERE category = 'GP Practice' AND icb_name IS NOT NULL
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
WHERE entity_type = 'site' AND trust_name IS NOT NULL
GROUP BY 1 ORDER BY 2 DESC LIMIT 3;
-- TEES, ESK AND WEAR VALLEYS NHS FOUNDATION TRUST       1121
-- MIDLANDS PARTNERSHIP UNIVERSITY NHS FOUNDATION TRUST   930
-- HAMPSHIRE AND ISLE OF WIGHT HEALTHCARE NHS FT          812
```

A NULL means ODS records no such relationship, not that we failed to resolve it.
Go to `relationships.parquet` when you need the relationship's own dates or
status.

## What is this old code now

```sql
SELECT ods_code, name FROM 'ods_data/current/orgs.parquet'
WHERE list_contains(predecessor_codes, '0AF');
-- YDDTR | NHS GREATER MANCHESTER SHARED SERVICES
```

`predecessor_codes` is the transitive closure, so one line of SQL does what would
otherwise be a recursive walk. Because `orgs.parquet` is active-only you get the
live descendants... run it against `orgs_all.parquet` to include the ones that
have since closed themselves. See [parquet.md] for why there's no single
"terminal successor".

## Use the operational dates

ODS carries two date families and you want `operational_*` nearly always. They're
the ones that are actually there:

- `operational_start` — every row, all 305,541 of them
- `operational_end` — every inactive row, plus 603 active ones with a closure
  already scheduled _(the furthest out is 2028-08-31)_
- `legal_start` — 14% of rows. 5.8% of active ones.
- `legal_end` — 12% of rows

So `status = 'active'` is the test for open today, not `operational_end IS NULL`,
which drops those 603.

**For experts only.** Legal dates are reliable for organisations created by
statute, and useless for everyone else, because a GP partnership or a corner shop
pharmacy has an opening day but no Act of Parliament. It shows in the fill rate:

| category | rows | has `legal_start` |
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

Some dates are placeholders. 12,470 rows have an `operational_start` of
`1900-01-01`, mostly schools. It's filler, not a claim about 1900.

```sql
SELECT count(*) FROM 'ods_data/current/orgs_all.parquet'
WHERE operational_start = DATE '1900-01-01';
```

636 active entities have no relationships at all, so an inner join against
`relationships.parquet` quietly drops them:

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

Every table carries `publication_date`, so a directory of pinned releases is a
time series and one glob reads the lot.

```sql
SELECT publication_date, count(*) AS n_rows,
       count(*) FILTER (WHERE status = 'active') AS n_active
FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
GROUP BY 1 ORDER BY 1;
```

| publication_date | n_rows | n_active |
| :--- | ---: | ---: |
| 2026-05-26 | 303,756 | 216,564 |
| 2026-06-22 | 304,662 | 216,417 |
| 2026-07-28 | 305,541 | 216,886 |

Nearly everything below is the same idiom with a different column plugged in:

```sql
lag(anything) OVER (PARTITION BY ods_code ORDER BY publication_date)
```

Three releases scan in under 0.1s, so this stays snappy well past a decade of
monthly snapshots.

## What opened and closed

```sql
WITH s AS (
  SELECT ods_code, name, publication_date, status,
         lag(status) OVER (PARTITION BY ods_code ORDER BY publication_date) AS prev_status
  FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
)
SELECT publication_date, prev_status, status, count(*) AS n
FROM s WHERE prev_status IS NOT NULL AND prev_status <> status
GROUP BY ALL ORDER BY 1, 2;
```

| publication_date | was | now | n |
| :--- | :--- | :--- | ---: |
| 2026-06-22 | active | inactive | 1068 |
| 2026-06-22 | inactive | active | 17 |
| 2026-07-28 | active | inactive | 618 |
| 2026-07-28 | inactive | active | 43 |

The reopenings are the half nobody expects. 60 organisations came back from the
dead over two months, and no single release records that it ever happened.

## The archive is the only complete list

165 codes that were in the May release are gone from July. Not closed — gone.
Every one of them an inactive organisation when last seen, mostly schools.

```sql
WITH r AS (SELECT ods_code, publication_date FROM read_parquet('ods_data/releases/*/orgs_all.parquet'))
SELECT count(*) FROM (
  SELECT ods_code FROM r GROUP BY 1 HAVING max(publication_date) < DATE '2026-07-28'
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
  SELECT ods_code, name, publication_date,
         lag(name) OVER (PARTITION BY ods_code ORDER BY publication_date) AS was
  FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
)
SELECT ods_code, was, name AS now, publication_date
FROM s WHERE was IS NOT NULL AND name <> was ORDER BY ods_code;
```

| ods_code | was | now |
| :--- | :--- | :--- |
| 8HQ46 | CSAM (UK) LTD | OMDA HEALTH ANALYTICS LIMITED |
| 8JH56 | AGE CONCERN BIRMINGHAM | AGE CONNECT WEST MIDLANDS |
| 8HR16 | BANTHAM TECHNOLOGIES LIMITED | INKWRX LIMITED |

330 renames in June, 320 in July, and 439 then 407 postcode changes. Swap `name`
for `address`, `town` or `uprn` to watch things move house.

## Why a pinned release beats a date filter

Two ways to ask what was active on `2026-05-26` — read the May snapshot, or
filter July's data by date.

```sql
SELECT
 (SELECT count(*) FROM 'ods_data/releases/2026-05-29/orgs_all.parquet'
   WHERE status = 'active') AS as_ods_knew_it,
 (SELECT count(*) FROM 'ods_data/releases/2026-07-31/orgs_all.parquet'
   WHERE operational_start <= DATE '2026-05-26'
     AND (operational_end IS NULL OR operational_end > DATE '2026-05-26')) AS reconstructed;
-- 216,564 | 216,570
```

Six apart. Looks like the reconstruction works. It doesn't — compare the sets
rather than the counts:

```sql
WITH may AS (SELECT ods_code FROM 'ods_data/releases/2026-05-29/orgs_all.parquet' WHERE status = 'active'),
     jul AS (SELECT ods_code FROM 'ods_data/releases/2026-07-31/orgs_all.parquet'
              WHERE operational_start <= DATE '2026-05-26'
                AND (operational_end IS NULL OR operational_end > DATE '2026-05-26'))
SELECT (SELECT count(*) FROM (SELECT * FROM may EXCEPT SELECT * FROM jul)) AS snapshot_only,
       (SELECT count(*) FROM (SELECT * FROM jul EXCEPT SELECT * FROM may)) AS reconstruction_only;
-- 318 | 324
```

642 organisations wrong, netting out to 6, at two months' distance. Mostly that's
ordinary churn rather than history being rewritten — records that were already
closed got their end dates edited just 1 time in June and 7 in July:

```sql
WITH s AS (
  SELECT ods_code, publication_date, status, operational_end, legal_end,
         lag(status)          OVER w AS p_status,
         lag(operational_end) OVER w AS p_op_end,
         lag(legal_end)       OVER w AS p_lg_end
  FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
  WINDOW w AS (PARTITION BY ods_code ORDER BY publication_date)
)
SELECT publication_date,
       count(*) FILTER (WHERE operational_end IS DISTINCT FROM p_op_end) AS op_end_rewritten,
       count(*) FILTER (WHERE legal_end       IS DISTINCT FROM p_lg_end) AS legal_end_rewritten
FROM s WHERE p_status = 'inactive' AND status = 'inactive'
GROUP BY 1 ORDER BY 1;
```

Both effects are real, they're just very different sizes. Either way, if you want
the register as ODS knew it on a date, use the release from that date.

## Reparenting

```sql
WITH s AS (
  SELECT ods_code, publication_date,
         parent_code, icb_code, trust_code, pcn_code,
         lag(parent_code) OVER w AS p_parent, lag(icb_code) OVER w AS p_icb,
         lag(trust_code)  OVER w AS p_trust,  lag(pcn_code) OVER w AS p_pcn
  FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
  WINDOW w AS (PARTITION BY ods_code ORDER BY publication_date)
)
SELECT publication_date,
  count(*) FILTER (WHERE parent_code IS DISTINCT FROM p_parent) AS parent_changed,
  count(*) FILTER (WHERE trust_code  IS DISTINCT FROM p_trust)  AS trust_changed,
  count(*) FILTER (WHERE icb_code    IS DISTINCT FROM p_icb)    AS icb_changed,
  count(*) FILTER (WHERE pcn_code    IS DISTINCT FROM p_pcn)    AS pcn_changed
FROM s WHERE publication_date > DATE '2026-05-26'
GROUP BY 1 ORDER BY 1;
```

| publication_date | parent | trust | icb | pcn |
| :--- | ---: | ---: | ---: | ---: |
| 2026-06-22 | 1542 | 406 | 253 | 6 |
| 2026-07-28 | 1275 | 541 | 281 | 6 |

Anything you aggregate by ICB or trust over a year is aggregating a hierarchy
that moved underneath you. PCN membership barely budges. Parentage never sits
still.

## Is `last_changed` honest?

ODS publishes a `last_changed` date on every record. With an archive you can
check whether it's telling the truth, instead of hoping.

```sql
WITH s AS (
  SELECT ods_code, publication_date, last_changed, name, postcode, status, primary_role_code,
         lag(last_changed)      OVER w AS p_changed,  lag(name)     OVER w AS p_name,
         lag(postcode)          OVER w AS p_postcode, lag(status)   OVER w AS p_status,
         lag(primary_role_code) OVER w AS p_role
  FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
  WINDOW w AS (PARTITION BY ods_code ORDER BY publication_date)
), c AS (
  SELECT *,
    (name IS DISTINCT FROM p_name OR postcode IS DISTINCT FROM p_postcode
     OR status IS DISTINCT FROM p_status OR primary_role_code IS DISTINCT FROM p_role) AS edited,
    (last_changed IS DISTINCT FROM p_changed) AS stamped
  FROM s WHERE p_changed IS NOT NULL
)
SELECT publication_date,
  count(*) FILTER (WHERE edited AND stamped)     AS edited_and_stamped,
  count(*) FILTER (WHERE edited AND NOT stamped) AS edited_not_stamped,
  count(*) FILTER (WHERE NOT edited AND stamped) AS stamped_no_visible_edit
FROM c GROUP BY 1 ORDER BY 1;
```

| publication_date | edited & stamped | edited, not stamped | stamped, no visible edit |
| :--- | ---: | ---: | ---: |
| 2026-06-22 | 1796 | 0 | 1071 |
| 2026-07-28 | 1334 | 0 | 1730 |

Not one unstamped edit in either month, across name, postcode, status and primary
role. That's a good result for ODS. _(It only covers those four fields over two
months, mind. The right hand column is edits to roles and relationships, which
don't show up as `orgs` columns.)_

## Successions turn up late

```sql
WITH s AS (
  SELECT ods_code, publication_date, successor_codes,
         lag(successor_codes) OVER (PARTITION BY ods_code ORDER BY publication_date) AS was
  FROM read_parquet('ods_data/releases/*/orgs_all.parquet')
)
SELECT publication_date, count(*) AS gained_successors
FROM s WHERE was IS NOT NULL AND len(successor_codes) > len(was)
GROUP BY 1 ORDER BY 1;
-- 2026-07-28 | 3
```

The paperwork lands after the merger, so a mapping built from one release can be
incomplete for anything recent.

## Drift in `category` might be us, not ODS

`category` changed on 1 row in June and 2 in July. It's our opinion though, not
ODS data, so check the rules didn't move before you report it as change in the
NHS:

```console
$ shasum -a 256 ods_data/releases/*/category_rules.json
397842a288c028917e1741d73a961c68bfd6f2d9b7fc1e5858c0a71535c3e884  ods_data/releases/2026-05-29/category_rules.json
397842a288c028917e1741d73a961c68bfd6f2d9b7fc1e5858c0a71535c3e884  ods_data/releases/2026-06-26/category_rules.json
397842a288c028917e1741d73a961c68bfd6f2d9b7fc1e5858c0a71535c3e884  ods_data/releases/2026-07-31/category_rules.json
```

Same hash, so the classification held still and the change came from upstream.
That's why [category_rules.json] ships inside every release.

## Traps

**`releases/*/orgs*.parquet` counts everything twice.** The glob matches
`orgs.parquet` _and_ `orgs_all.parquet` in every directory: 1,563,826 rows where
there are really 649,867. `orgs` is a subset of `orgs_all` by design. Name the
file you actually want.

**Group by `publication_date`, never by the directory name.** They're different
dates. The `2026-05-29/` release has publication date `2026-05-26` — the
directory is when TRUD distributed it, the column is when ODS generated it.

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

[parquet.md]: ./parquet.md
[category_rules.json]: ../data/category_rules.json
