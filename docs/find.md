# `ods find` Command Design & Guiding Principles

This document defines the Developer Experience (DX) guiding principles, query model, output formats, and schema parity specifications for `ods find`.

---

## 1. Guiding Principle & Value Proposition

> **"`ods find` is the instant, zero-friction, domain-aware inspector for NHS Organisation Data."**

### Key Guiding Rules
1. **Single Source of Truth**: `ods find` queries `orgs.parquet`, `orgs_all.parquet`, `successors.parquet`, and `roles.parquet`.
2. **Schema Parity Across Formats**: `ods find --format json` and `ods find --format csv` reuse the exact field names and sequence of `orgs.parquet` with zero truncation, placing enriching properties in logical concept groups.
3. **Verbose Inspector Mode (`--verbose`) & Wiki Generator (`ods md`)**: Share a unified inspector rendering engine (`render_inspector_markdown`). Formats compact range intervals (`Operational: <start> to present` / `<start> to <end>`), `Other Roles`, `Last Modified` timestamp, contact details, and complete hierarchy relationship links (`PCN`, `Trust`, `ICB`, `Region`, `Commissioned By`, `Parent Org`, `Succeeded By`).

---

## 2. Full Machine Readable Formats (`--format json` & `--format csv`)

Every entity returned by `ods find --format json` and `ods find --format csv` exports the complete 34-field schema:

```
ods_code,record_class,status,role,role_code,secondary_roles,name,address,town,county,postcode,country,uprn,telephone,website,commissioner,commissioner_code,parent,parent_code,pcn,pcn_code,trust,trust_code,icb,icb_code,region,region_code,successor_code,successor,legal_start,legal_end,operational_start,operational_end,last_change_date
```

### JSON Structure (`--format json`)
```json
{
  "ods_code": "A82608",
  "record_class": "org",
  "status": "active",
  "role": "prescribing cost centre",
  "role_code": "RO177",
  "secondary_roles": ["gp practice (RO76)"],
  "name": "SEDBERGH MEDICAL PRACTICE",
  "address": "STATION ROAD, SEDBERGH, LA10 5DL",
  "town": "SEDBERGH",
  "county": "CUMBRIA",
  "postcode": "LA10 5DL",
  "country": "ENGLAND",
  "uprn": "10003970417",
  "telephone": "01539 718191",
  "website": null,
  "commissioner": "NHS LANCASHIRE AND SOUTH CUMBRIA INTEGRATED CARE BOARD",
  "commissioner_code": "QE1",
  "parent": "NHS LANCASHIRE AND SOUTH CUMBRIA ICB - 01K",
  "parent_code": "01K",
  "pcn": "WESTERN DALES PCN",
  "pcn_code": "U59980",
  "trust": null,
  "trust_code": null,
  "icb": "NHS LANCASHIRE AND SOUTH CUMBRIA INTEGRATED CARE BOARD",
  "icb_code": "QE1",
  "region": null,
  "region_code": null,
  "successor_code": null,
  "successor": null,
  "legal_start": null,
  "legal_end": null,
  "operational_start": "1987-04-01",
  "operational_end": null,
  "last_change_date": "2023-08-22"
}
```

---

## 3. Extended Inspector Mode (`--verbose` & `ods md`)

When invoked with `--verbose` (or when a single entity is matched in table format), `ods find` outputs an ANSI-colored Markdown summary via the shared `formatting::render_inspector_markdown` renderer:

```markdown
# SEDBERGH MEDICAL PRACTICE (A82608)

- Class: org
- Status: active
- Primary Role: prescribing cost centre (RO177)
- Other Roles: gp practice (RO76)
- Operational: 1987-04-01 to present
- Last Modified: 2023-08-22

## Contact Details
- Address: STATION ROAD, SEDBERGH, LA10 5DL
- Country: ENGLAND
- UPRN: 10003970417
- Telephone: 01539 718191

## Relationships
- PCN: WESTERN DALES PCN (U59980)
- ICB: NHS LANCASHIRE AND SOUTH CUMBRIA INTEGRATED CARE BOARD (QE1)
- Commissioned By: NHS LANCASHIRE AND SOUTH CUMBRIA INTEGRATED CARE BOARD (QE1)
- Parent Org: NHS LANCASHIRE AND SOUTH CUMBRIA ICB - 01K (01K)
```

---

## 4. Dynamic Parquet $\leftrightarrow$ JSON & CSV Schema Parity Tests

To guarantee schema parity across formats, `src/commands/find.rs` includes two dynamic tests:
- `test_find_json_schema_matches_parquet_schema`: Verifies key sequence parity with `orgs.parquet`.
- `test_find_csv_schema_matches_json_schema`: Verifies that CSV headers match JSON keys line-for-line.

