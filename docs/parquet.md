# ODS Parquet Schema Design

This schema optimizes the NHS ODS XML dataset into a highly ergonomic, local-first analytical data product. It prioritizes 80/20 usability, allowing analysts to answer 95% of administrative, geographical, and hierarchical queries using a single, zero-join table, while preserving complete relational integrity in secondary files for specialized use cases.

The ODS relationship graph is simplified and split into **3 Parquet files**:

*   **`orgs.parquet`** (The Core Table):
    This is the primary table that analysts should use for the vast majority (95%+) of queries. It contains the flat, current state of all organisations and sites, denormalizing the physical addresses, electronic contact details, and recursively resolved parent hierarchies directly onto each entity's row. Joins on `ods_code` (Primary Key) are not required for standard analytical tasks. The rows are sorted by `status ASC` (Active first) and `ods_code ASC` to optimize search performance.

*   **`roles.parquet`** (Supporting Table):
    This supporting table maps ODS codes (Foreign Key) to their complete historic timeline of administrative roles (e.g. tracking when an organization changed its primary function). It is only needed when validating historical roles held by an organisation. Sorted by `ods_code ASC, operational_start DESC` to keep timelines newest-first.

*   **`rels.parquet`** (Supporting Table):
    This supporting table acts as a relationship graph ledger mapping all generic, non-hierarchical, or historical links between entities (such as successions, commissioning changes, or specialized clinical networks) via `source_code` and `target_code` (Foreign Keys). Sorted by `target_code ASC, rel_type ASC` to cluster incoming relations.

---

## Schema (`orgs.parquet`)

This table is arranged left-to-right to optimize both human visual ergonomics (for CLI / `SELECT *` scans) and Parquet column-chunk compression logic.

### Columns
| Column Name | Data Type | Nullable? | Example Value / Operational Context |
| :--- | :--- | :--- | :--- |
| `ods_code` | VARCHAR | No | `"A101"` (Primary Key / Explicit Identity) |
| `status` | VARCHAR | No | `"Active"` or `"Inactive"` |
| `role` | VARCHAR | No | `"GP PRACTICE"` (Mapped from role code) |
| `name` | VARCHAR | No | `"THE BROOK SURGERY"` |
| `address` | VARCHAR | Yes | `"12 GRESHAM ROAD, SUITE 4, BUSINESS PARK, LONDON, GREATER LONDON, SW2 1EG"` (Consolidated address string) |
| `town` | VARCHAR | Yes | `"LONDON"` (Standard Post Town format) |
| `county` | VARCHAR | Yes | `"GREATER LONDON"` |
| `postcode` | VARCHAR | Yes | `"SW2 1EG"` |
| `country` | VARCHAR | Yes | `"ENGLAND"` |
| `uprn` | VARCHAR | Yes | `"100062506311"` (Unique Property Reference Number) |
| `telephone` | VARCHAR | Yes | `"023 80706919"` |
| `website` | VARCHAR | Yes | `"http://example.com"` |
| `parent` | VARCHAR | Yes | `"SOUTH LONDON HEALTH PARTNERS"` |
| `pcn` | VARCHAR | Yes | `"BRIXTON & HERNE HILL PCN"` |
| `trust` | VARCHAR | Yes | `"GUY'S AND ST THOMAS' NHS FOUNDATION TRUST"` |
| `icb` | VARCHAR | Yes | `"NHS SOUTH EAST LONDON ICB"` |
| `role_code` | VARCHAR | No | `"RO177"` (Standard ODS role identifier) |
| `parent_code` | VARCHAR | Yes | `"Y04321"` (Immediate administrative parent) |
| `pcn_code` | VARCHAR | Yes | `"U12345"` (Primary Care Network Code) |
| `trust_code` | VARCHAR | Yes | `"RGT"` (Overarching Acute Trust Code) |
| `icb_code` | VARCHAR | Yes | `"15N"` (Integrated Care Board Code) |
| `legal_start` | DATE | Yes | `2013-04-01` (Legal start date) |
| `legal_end` | DATE | Yes | `2020-03-31` (Legal end date, NULL if active) |
| `operational_start` | DATE | Yes | `2013-04-01` (Operational start date) |
| `operational_end` | DATE | Yes | `2020-03-31` (Operational end date, NULL if active) |

---

## Design Choices

1. **Left-to-Right Identity & Diagnosis**:
   - Placing `ods_code`, `status`, and `role` immediately at Columns 1-3 establishes *what* the entity is and *whether it is open* instantly.
2. **The Mailing Label & Contact Block**:
   - Grouping `name`, `address`, `town`, `county`, `postcode`, and `country` together replicates how humans naturally read addresses. Pushing telephone and website right next to the address groups all physical/electronic contact methods logically. Fax is omitted as it is obsolete.
3. **Micro-to-Macro Hierarchy Zoom**:
   - The hierarchy columns are ordered strictly by organizational scale: `parent` (direct parent) -> `pcn` (neighborhood) -> `trust` (provider) -> `icb` (regional system level).
4. **Code Separation / Human Bunched**:
   - All descriptive names are bunched on the left, while technical codes (`*_code`) are pushed to the right. This keeps `SELECT *` outputs beautiful in the terminal while maintaining full database join capability behind the scenes.
5. **No Made-Up Data**:
   - Alternative address tables are discarded because the ODS XML schema only defines a single geographical location (`GeoLoc`) per organisation record. Alternative mailing/billing locations do not exist in the source data.
6. **Consolidated Address Schema**:
   - Building name/number, street name, district, town, county, and postcode are consolidated into a single, clean `address` field (excluding country) to prevent street names from inconsistently hopping between multiple columns. This aligns with modern standards (like schema.org's `streetAddress`), provides an instant UI-ready mailing label, and simplifies full-text queries.
7. **Legal vs. Operational Dates**:
   - Instead of flattening timelines into a single start and end date (which results in massive data errors due to legal restructurings), we preserve both timelines as separate, explicit columns (`legal_start`/`end` and `operational_start`/`end`).

---

## Implementation Notes

1. **Dynamic CodeSystem Mapping**:
   - Rather than hardcoding static role mappings (which will fall out of sync when the NHS adds or renames roles), build the `concept_map` dynamically during **Pass 1** by parsing the `<CodeSystems>` definition block at the top of the TRUD XML file.
2. **Strict Sanitization (Whitespace & Case)**:
   - Call `.trim()` on all string fields to remove accidental padding (e.g. `"LONDON   " -> "LONDON"`).
   - Enforce UPPERCASE formatting on `town` to ensure consistent SQL filtering.
3. **Recursive Hierarchy Resolution**:
   - As relationships (`Rel` nodes) in the XML are simple source-to-target links, construct an in-memory graph during **Pass 1** mapping ODS codes to their active relationships and primary role codes.
   - During **Pass 2**, walk this graph to resolve higher-level parents:
     - Follow `RE4` (is commissioned by) $\rightarrow$ check if a parent holds role `RO318` $\rightarrow$ `icb`/`code`.
     - Check if a parent holds role `RO272` $\rightarrow$ `pcn`/`code`.
     - Check if a parent holds role `RO197` $\rightarrow$ `trust`/`code`.
4. **Timeline Date Extraction**:
   - Parse and extract both the `Legal` and `Operational` start and end dates from the dates list to populate the respective fields in the Parquet tables.

---

## Supporting Schemas

### Supporting Table 1: `roles.parquet` (The Role History Timeline)
- **Description**: The complete historic timeline of every role an entity has held.
- **Sorting Protocol**: `ods_code ASC, operational_start DESC` (timeline ordered newest first).

| Column Name | Data Type | Nullable? | Example Value / Context |
| :--- | :--- | :--- | :--- |
| `role` | VARCHAR | No | `"GP PRACTICE"` (Human-readable string) |
| `is_primary` | BOOLEAN | No | `TRUE` or `FALSE` (Quick flag for main role) |
| `status` | VARCHAR | No | `"Active"` or `"Inactive"` |
| `ods_code` | VARCHAR | No | `"A101"` (Foreign Key link to core table) |
| `role_code` | VARCHAR | No | `"RO177"` (Standard ODS role identifier) |
| `legal_start` | DATE | Yes | `2013-04-01` |
| `legal_end` | DATE | Yes | `NULL` (Populated if role was retired legally) |
| `operational_start` | DATE | Yes | `2013-04-01` |
| `operational_end` | DATE | Yes | `NULL` (Populated if role was retired operationally) |

### Supporting Table 2: `rels.parquet` (The Relationship Graph Ledger)
- **Description**: Master graph mapping historical, secondary, or complex linkages.
- **Sorting Protocol**: `target_code ASC, rel_type ASC` (clusters incoming paths).

| Column Name | Data Type | Nullable? | Example Value / Context |
| :--- | :--- | :--- | :--- |
| `rel_type` | VARCHAR | No | `"IS COMMISSIONED BY"` |
| `status` | VARCHAR | No | `"Active"` or `"Inactive"` |
| `target` | VARCHAR | Yes | `"NHS SOUTH EAST LONDON ICB"` |
| `source` | VARCHAR | Yes | `"THE BROOK SURGERY"` |
| `target_code` | VARCHAR | No | `"15N"` (The entity receiving the link) |
| `source_code` | VARCHAR | No | `"A101"` (The entity initiating the link) |
| `rel_type_code` | VARCHAR | No | `"RE4"` (Standard ODS relationship code) |
| `legal_start` | DATE | Yes | `2020-04-01` |
| `legal_end` | DATE | Yes | `NULL` (Populated if link is dead legally) |
| `operational_start` | DATE | Yes | `2020-04-01` |
| `operational_end` | DATE | Yes | `NULL` (Populated if link is dead operationally) |