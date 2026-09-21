# ods info

Show full details for a single organisation by ODS code.

`ods info` looks up an organisation by its exact ODS code in the active release dataset and renders its complete record, including primary and secondary roles, contact details, parent and commissioning relationships, and resolved hop-by-hop succession chains.

## Usage

```text
ods info <ODS_CODE> [OPTIONS]
```

### Options

| Flag | Description |
| :--- | :--- |
| `<ODS_CODE>` | Exact ODS organisation code (case-insensitive, e.g. `A82608`, `RJZ`, `0AF`) |
| `-f, --format <FORMAT>` | Output format: `markdown` (default) or `json` |
| `-i, --input <DIR>` | Directory containing Parquet files (defaults to active release in `ods_data/current`) |
| `-h, --help` | Print help information |

## Examples

### Inspect an active GP practice in Markdown

```bash
ods info A82608
```

Output:

```text
# SEDBERGH MEDICAL PRACTICE (A82608)
- Class: org
- Status: active
- Primary Role: Prescribing Cost Centre (RO177)
- Other Roles: GP Practice (RO76)
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
- Commissioned By: NHS LANCASHIRE AND SOUTH CUMBRIA ICB - 01K (01K)
- Parent Org: NHS LANCASHIRE AND SOUTH CUMBRIA ICB - 01K (01K)
```

### Inspect an inactive organisation with succession history in JSON

```bash
ods info 0AF --format json
```

Output:

```json
{
  "ods_code": "0AF",
  "name": "NORTH WEST REGIONAL HEALTH AUTHORITY",
  "record_class": "org",
  "status": "inactive",
  "primary_role_code": "RO137",
  "primary_role_name": "Regional Health Authority",
  "role_codes": [
    "RO137"
  ],
  "role_names": [
    "Regional Health Authority"
  ],
  "other_roles": [],
  "address": "930-932 WING C, FIRST FLOOR, BROWNLOW HILL, LIVERPOOL, L3 5TG",
  "town": "LIVERPOOL",
  "county": "MERSEYSIDE",
  "postcode": "L3 5TG",
  "country": "ENGLAND",
  "uprn": "10008139591",
  "telephone": "0151 2854000",
  "website": null,
  "succession": [
    {
      "depth": 1,
      "date": "2012-10-01",
      "code": "0CE",
      "name": "MERSEY CLUSTER",
      "status": "inactive"
    },
    {
      "depth": 2,
      "date": "2014-10-01",
      "code": "0CY",
      "name": "NHS NORTH (LANCASHIRE AND GREATER MANCHESTER)",
      "status": "inactive"
    },
    {
      "depth": 3,
      "date": "2016-04-01",
      "code": "YDDTR",
      "name": "NHS NORTH EAST AND YORKSHIRE INTEGRATED CARE BOARD",
      "status": "active"
    }
  ],
  "predecessors": [],
  "legal_start": "1994-04-01",
  "legal_end": "2012-10-01",
  "operational_start": "1994-04-01",
  "operational_end": "2012-10-01",
  "last_changed": "2013-03-22",
  "trud_release_date": "2026-07-31"
}
```

## How It Works

`ods info` searches `orgs.parquet`, which holds active and inactive/closed organisations alike. `--all` on `ods info` adds an organisation's inactive roles and relationships to the card; the organisation is found either way. When found, it traverses `successions.parquet` to construct forward and reverse succession paths with hop depths and effective dates.

- **Markdown mode**: Emits human-readable ANSI colored output when connected to a terminal, or clean plain markdown when piped.
- **JSON mode**: Emits structured JSON suitable for scripts, pipelines, and agents.
