# `orgs.parquet` Schema & Design Specification

This document defines the Arrow/Parquet schema, field definitions, and denormalization rules for `orgs.parquet` and `orgs_all.parquet` produced by `ods parquet`.

---

## 1. Overview

`orgs.parquet` is the primary, single-table analytical surface for NHS Organisation Data. It denormalizes core identity, classification, geographic locations, contact information, administrative hierarchies, operational date ranges, and modification metadata into a flat, high-performance table.

- **`orgs.parquet`**: Contains active entities (`status = "active"`).
- **`orgs_all.parquet`**: Contains the full historical dataset (`status = "active"` and `status = "inactive"`).

---

## 2. Complete Field Specification (31 Columns)

| Field Name | Parquet Data Type | Nullable | Description & Example |
| :--- | :--- | :--- | :--- |
| `ods_code` | VARCHAR | No | Primary Key / ODS Code (e.g. `"A82608"`) |
| `record_class` | VARCHAR | No | Entity Record Class (`"org"` or `"site"`) |
| `status` | VARCHAR | No | Operational Status (`"active"` or `"inactive"`) |
| `role` | VARCHAR | No | Primary Role display name (e.g. `"prescribing cost centre"`) |
| `role_code` | VARCHAR | No | Primary Role ODS Identifier (e.g. `"RO177"`) |
| `name` | VARCHAR | No | Organisation / Site Name (e.g. `"SEDBERGH MEDICAL PRACTICE"`) |
| `address` | VARCHAR | Yes | Consolidated street address string |
| `town` | VARCHAR | Yes | Post Town (e.g. `"SEDBERGH"`) |
| `county` | VARCHAR | Yes | County (e.g. `"CUMBRIA"`) |
| `postcode` | VARCHAR | Yes | Postcode (e.g. `"LA10 5DL"`) |
| `country` | VARCHAR | Yes | Country (e.g. `"ENGLAND"`) |
| `uprn` | VARCHAR | Yes | Unique Property Reference Number |
| `telephone` | VARCHAR | Yes | Main Telephone Number |
| `website` | VARCHAR | Yes | Web Address URL |
| `commissioner` | VARCHAR | Yes | Commissioning Body Name (e.g. `"NHS LANCASHIRE AND SOUTH CUMBRIA ICB"`) |
| `commissioner_code` | VARCHAR | Yes | Commissioning Body ODS Code (e.g. `"QE1"`) |
| `parent` | VARCHAR | Yes | Immediate Administrative Parent Name |
| `parent_code` | VARCHAR | Yes | Immediate Administrative Parent ODS Code (e.g. `"01K"`) |
| `pcn` | VARCHAR | Yes | Primary Care Network Name (e.g. `"WESTERN DALES PCN"`) |
| `pcn_code` | VARCHAR | Yes | Primary Care Network ODS Code (e.g. `"U59980"`) |
| `trust` | VARCHAR | Yes | Overarching Acute / Foundation Trust Name |
| `trust_code` | VARCHAR | Yes | Overarching Trust ODS Code |
| `icb` | VARCHAR | Yes | Integrated Care Board Name |
| `icb_code` | VARCHAR | Yes | Integrated Care Board ODS Code |
| `region` | VARCHAR | Yes | NHS England Regional Directorate Name |
| `region_code` | VARCHAR | Yes | NHS England Regional Directorate Code (e.g. `"Y62"`) |
| `legal_start` | DATE | Yes | Legal Start Date (`YYYY-MM-DD`) |
| `legal_end` | DATE | Yes | Legal End Date (`YYYY-MM-DD`, NULL if active) |
| `operational_start` | DATE | Yes | Operational Start Date (`YYYY-MM-DD`) |
| `operational_end` | DATE | Yes | Operational End Date (`YYYY-MM-DD`, NULL if active) |
| `last_change_date` | DATE | Yes | Last Record Modification Date by NHS Digital (`YYYY-MM-DD`) |