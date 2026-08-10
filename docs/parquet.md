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

---

## 3. `rels.parquet` Schema Specification (11 Columns)

`rels.parquet` models the directed relationship graph between NHS entities (e.g. commissioning, operating, location, and payee links).

| Field Name | Parquet Data Type | Nullable | Description & Example |
| :--- | :--- | :--- | :--- |
| `source_code` | VARCHAR | Yes | Subject / Source Entity ODS Code (e.g. `"A82608"`) |
| `source` | VARCHAR | Yes | Subject / Source Entity Name |
| `rel_type_code` | VARCHAR | Yes | Relationship Type Code (e.g. `"RE4"`, `"RE6"`, `"RE5"`) |
| `rel_type` | VARCHAR | Yes | Relationship Type Description (e.g. `"is commissioned by"`, `"is operated by"`, `"is located in the geography of"`) |
| `target_code` | VARCHAR | Yes | Target Entity ODS Code (e.g. `"QMJ"`) |
| `target` | VARCHAR | Yes | Target Entity Name (e.g. `"NHS NORTH CENTRAL LONDON INTEGRATED CARE BOARD"`) |
| `status` | VARCHAR | Yes | Relationship Status (`"active"` or `"inactive"`) |
| `legal_start` | DATE | Yes | Relationship Legal Start Date |
| `legal_end` | DATE | Yes | Relationship Legal End Date |
| `operational_start` | DATE | Yes | Relationship Operational Start Date |
| `operational_end` | DATE | Yes | Relationship Operational End Date |

### Primary Relationship Types in TRUD Data:
- **`RE4` (`is commissioned by`)**: 148,000+ links connecting GP Practices, PCNs, Pharmacies, and Dentists to their Commissioning ICB / Sub-ICB.
- **`RE6` (`is operated by`)**: 202,000+ links connecting Hospital Sites, Clinics, and Care Units to their Operating NHS Trust or Provider.
- **`RE5` (`is located in the geography of`)**: 292,000+ links connecting entities to Government Office Regions, Local Authorities, or Local Health Districts.

---

## 4. `roles.parquet` Schema Specification (9 Columns)

`roles.parquet` contains all primary and secondary role assignments for every organisation in the dataset.

| Field Name | Parquet Data Type | Nullable | Description & Example |
| :--- | :--- | :--- | :--- |
| `ods_code` | VARCHAR | Yes | Entity ODS Code (e.g. `"QMJ"`) |
| `role_code` | VARCHAR | Yes | Role ODS Code Identifier (e.g. `"RO318"`, `"RO261"`) |
| `role` | VARCHAR | Yes | Role Display Name (e.g. `"integrated care board"`, `"strategic partnership"`) |
| `is_primary` | BOOLEAN | Yes | `true` if primary role, `false` if secondary role |
| `status` | VARCHAR | Yes | Role Status (`"active"` or `"inactive"`) |
| `legal_start` | DATE | Yes | Role Legal Start Date |
| `legal_end` | DATE | Yes | Role Legal End Date |
| `operational_start` | DATE | Yes | Role Operational Start Date |
| `operational_end` | DATE | Yes | Role Operational End Date |

---

## 5. `successors.parquet` Schema Specification (5 Columns)

`successors.parquet` contains the structural succession graph for re-organized, merged, or split NHS bodies.

| Field Name | Parquet Data Type | Nullable | Description & Example |
| :--- | :--- | :--- | :--- |
| `ods_code` | VARCHAR | Yes | Original / Predecessor Entity ODS Code |
| `name` | VARCHAR | Yes | Original / Predecessor Entity Name |
| `successor_code` | VARCHAR | Yes | Successor Entity ODS Code |
| `successor` | VARCHAR | Yes | Successor Entity Name |
| `succession_chain` | VARCHAR | Yes | Complete formatted lineage chain string (e.g. `"01K -> 93C -> QMJ"`) |