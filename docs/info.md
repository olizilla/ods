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
| `-f, --format <FORMAT>` | Output format: `table` (default), `markdown`, or `json` |
| `-i, --input <DIR>` | Directory containing Parquet files (defaults to active release) |
| `-a, --all` | Show all roles and relationships, including closed or inactive |
| `-w, --width <WIDTH>` | Override terminal width for responsive layout (e.g. `58`, `78`, `98`) |
| `--plain` | Disable ANSI coloured output |
| `-h, --help` | Print help |

## Examples

### Look up an organisation

`table` is the default format: a responsive card, coloured when stdout is a terminal.

```console
$ ods info A81002
* Source: releases/2026-08-28/orgs.parquet
* Status: inactive, Class: org, Last Change: 2019-04-01
┌──────────────┬─────────────────────────────────────────────────────────────────────────────────┐
│ ODS Code     ┆ A81002                                                                          │
╞══════════════╪═════════════════════════════════════════════════════════════════════════════════╡
│ Name         ┆ NORTHGATE MEDICAL PRACTICE                                                      │
│ Address      ┆ 12 Northgate, Darlington, DL1 1PS                                               │
│ Country      ┆ England                                                                         │
│ UPRN         ┆ 100110043872                                                                    │
│ Website      ┆ —                                                                               │
│ Telephone    ┆ —                                                                               │
│ Opened       ┆ 1974-04-01                                                                      │
│ Closed       ┆ 2019-03-31                                                                      │
│ Succeeded by ┆ A81003 RIVERSIDE MEDICAL GROUP                                                  │
└──────────────┴─────────────────────────────────────────────────────────────────────────────────┘

Roles (none active, 2 inactive)
┌──────────────────────────────────────────────────────────────┬───────┬────────────┬────────────┐
│ Role                                                         ┆ Code  ┆ Start      ┆ End        │
╞══════════════════════════════════════════════════════════════╪═══════╪════════════╪════════════╡
│ GP Practice                                                  ┆ RO76  ┆ 1974-04-01 ┆ 2019-03-31 │
│ Prescribing Cost Centre                                      ┆ RO177 ┆ 1974-04-01 ┆ 2019-03-31 │
└──────────────────────────────────────────────────────────────┴───────┴────────────┴────────────┘

Relationships (1 active, 4 inactive)
┌───┬─────────────────────┬───────────────────────────────────┬────────┬────────────┬────────────┐
│   ┆ Relationship        ┆ Organisation                      ┆ Code   ┆ Start      ┆ End        │
╞═══╪═════════════════════╪═══════════════════════════════════╪════════╪════════════╪════════════╡
│ ! ┆ commissioned by     ┆ NHS TEES VALLEY ICB               ┆ 00K    ┆ 2022-07-01 ┆ —          │
│   ┆                     ┆ NHS DARLINGTON CCG                ┆ 03D    ┆ 2013-04-01 ┆ 2019-03-31 │
│   ┆ in the geography of ┆ NHS DARLINGTON CCG                ┆ 03D    ┆ 2013-04-01 ┆ 2019-03-31 │
│ ! ┆ partner to          ┆ CENTRAL DARLINGTON PCN            ┆ U54321 ┆ 2019-07-01 ┆ 2019-03-31 │
│   ┆ nominated payee for ┆ ORCHARD GROUP PRACTICE            ┆ A81005 ┆ 2015-04-01 ┆ 2019-03-31 │
└───┴─────────────────────┴───────────────────────────────────┴────────┴────────────┴────────────┘

! 1 row still open on a closed organisation
! 1 row starts after the organisation closed
```

### `--format markdown`

The same record, always at the wide band and never coloured — for pasting somewhere
that doesn't render a terminal:

```console
$ ods info A81002 --format markdown
* Source: releases/2026-08-28/orgs.parquet
* Status: inactive, Class: org, Last Change: 2019-04-01

| ODS Code     | A81002                            |
|--------------|-----------------------------------|
| Name         | NORTHGATE MEDICAL PRACTICE        |
| Address      | 12 Northgate, Darlington, DL1 1PS |
| Country      | England                           |
| UPRN         | 100110043872                      |
| Website      | —                                 |
| Telephone    | —                                 |
| Opened       | 1974-04-01                        |
| Closed       | 2019-03-31                        |
| Succeeded by | A81003 RIVERSIDE MEDICAL GROUP    |

## Roles (none active, 2 inactive)

| Role                    | Code  | Start      | End        |
|-------------------------|-------|------------|------------|
| GP Practice             | RO76  | 1974-04-01 | 2019-03-31 |
| Prescribing Cost Centre | RO177 | 1974-04-01 | 2019-03-31 |

## Relationships (1 active, 4 inactive)

|   | Relationship        | Organisation           | Code   | Start      | End        |
|---|---------------------|------------------------|--------|------------|------------|
| ! | commissioned by     | NHS TEES VALLEY ICB    | 00K    | 2022-07-01 | —          |
|   |                     | NHS DARLINGTON CCG     | 03D    | 2013-04-01 | 2019-03-31 |
|   | in the geography of | NHS DARLINGTON CCG     | 03D    | 2013-04-01 | 2019-03-31 |
| ! | partner to          | CENTRAL DARLINGTON PCN | U54321 | 2019-07-01 | 2019-03-31 |
|   | nominated payee for | ORCHARD GROUP PRACTICE | A81005 | 2015-04-01 | 2019-03-31 |

> ! 1 row still open on a closed organisation

> ! 1 row starts after the organisation closed
```

### `--format json`

An inactive organisation with a three-hop succession chain:

```json
{
  "ods_code": "0AF",
  "name": "NORTH WEST REGIONAL HEALTH AUTHORITY",
  "status": "inactive",
  "record_class": "org",
  "role_codes": [
    "RO137"
  ],
  "role_names": [
    "Regional Health Authority"
  ],
  "primary_role_code": "RO137",
  "address": "930-932 WING C, FIRST FLOOR, BROWNLOW HILL, LIVERPOOL, L3 5TG",
  "town": "LIVERPOOL",
  "county": "MERSEYSIDE",
  "postcode": "L3 5TG",
  "country": "ENGLAND",
  "uprn": "10008139591",
  "telephone": "0151 2854000",
  "website": null,
  "predecessor_codes": [],
  "successor_codes": [
    "0CE",
    "0CY",
    "YDDTR"
  ],
  "legal_start": "1994-04-01",
  "legal_end": "2012-10-01",
  "operational_start": "1994-04-01",
  "operational_end": "2012-10-01",
  "last_changed": "2013-03-22",
  "publication_date": "2026-07-28",
  "primary_role_name": "Regional Health Authority",
  "other_roles": [],
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
  "predecessors": []
}
```

The field order is `orgs.parquet`'s own column order ([parquet.md](./parquet.md)), followed by
what `ods info` adds: `primary_role_name`, `other_roles`, `relationships` (present only when
non-empty), `succession` and `predecessors`.

## How it works

`ods info` uses `orgs.parquet`, which holds active and inactive organisations. `--all` adds an organisation's inactive roles and relationships to the card; the organisation is found either way. When found, it traverses `successions.parquet` to build forward and reverse succession paths, with hop depths and effective dates.

- **Table** (the default): a responsive card in three width bands, coloured when stdout is a terminal, plain when it isn't or under `--plain`.
- **Markdown**: the same record as Markdown tables, always at the wide band and never coloured.
- **JSON**: structured JSON suitable for scripts, pipelines and agents.
