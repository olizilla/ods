# NHS TRUD ODS XML Structure & Gotchas

This document details the internal structure, quirks, quality edge-cases, and parser gotchas of the official NHS Organisation Data Service (ODS) XML data distributions downloaded from TRUD (Technology Reference data Update Distribution).

---

## 1. Distribution & Archive Packaging

The NHS ODS data is published monthly on TRUD as a single root `.zip` archive (e.g., `hscorgrefdataxml_data_5.0.0_YYYYMMDD000001.zip`).

Inside this ZIP container are two core XML archives:

1. **`fullfile.zip`** (`HSCOrgRefData_Full_*.xml`):
   - Contains all currently known organisations, active and inactive top-level entities, primary/secondary role assignments, and current active/retired relationships.
   - Ingested as the baseline entity source for `orgs_all.parquet` and `orgs.parquet`.

2. **`archive.zip`** (`HSCOrgRefData_Archive_*.xml`):
   - Contains legacy historical organisations and closed organisation successor histories dating back decades.
   - Ingested by `ods parquet` to populate `successors.parquet` with complete historical successor chains (89,445 records total).

---

## 2. Core XML Element Hierarchy

The XML root element `<MANIFEST>` contains publication metadata (`publication_date`), followed by a flat stream of `<Organisation>` elements:

```xml
<MANIFEST>
  <PublicationDate value="2026-05-26" />
  
  <Organisation orgRecordClass="RC1">
    <OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" assigningAuthorityName="HSCIC" extension="RXT" />
    <Name>MARSDEN NHS FOUNDATION TRUST</Name>
    <Status value="Active" />
    <LastChangeDate value="2026-01-15" />
    
    <GeoLoc>
      <Location>
        <AddrLine1>FULHAM ROAD</AddrLine1>
        <Town>LONDON</Town>
        <PostCode>SW3 6JJ</PostCode>
        <Country>ENGLAND</Country>
      </Location>
    </GeoLoc>
    
    <Roles>
      <Role id="RO197" uniqueRoleId="101" status="Active">
        <Date><Type value="Operational" /><Start value="1991-04-01" /></Date>
      </Role>
    </Roles>
    
    <Rels>
      <Rel id="RE4" uniqueRelId="201" status="Active">
        <Target>
          <OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="Y58" />
        </Target>
      </Rel>
    </Rels>
    
    <Succs>
      <Succ type="Successor" uniqueSuccId="301">
        <Target>
          <OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="RXT01" />
        </Target>
      </Succ>
    </Succs>
  </Organisation>
</MANIFEST>
```

---

## 3. Structural Quirks & Parser Gotchas

During the development of `ods`, we identified several non-obvious structural quirks in NHS TRUD XML files. Any parser or auditing tool built for NHS ODS XML **must** account for these behaviors:

### Gotcha 1: Missing Top-Level `<Status>` Tags (NHS Region Offices)
- **The Issue**: Active NHS administrative bodies (such as NHS Region offices `Y58`, `Y59`, `Y60`, etc.) omit top-level `<Status value="Active">` elements in raw TRUD XML.
- **Why it matters**: A naive parser checking only top-level `<Status>` tags will categorize active NHS Region offices as inactive, dropping them from active analytical queries (`orgs.parquet`).
- **Resolution**: If top-level `<Status>` is missing, `ods` checks whether the primary role has `status="Active"`. If `primary_role.status == "Active"`, the entity is correctly treated as active.

---

### Gotcha 2: Scope Leakage from Nested `<Target><Organisation>` Nodes
- **The Issue**: Within relationship (`<Rel>`) and successor (`<Succ>`) tags, the XML embeds target organisation elements:
  ```xml
  <Rel id="RE4">
    <Target>
      <Organisation>
        <OrgId extension="5HN06" />
        <Name>SETT VALLEY MEDICAL CENTRE</Name> <!-- Target Name -->
      </Organisation>
    </Target>
  </Rel>
  ```
- **Why it matters**: 
  1. **SAX Scope Pollution**: A SAX XML parser reading `<Name>` without tracking nesting depth (`org_depth == 1`) or target context (`!in_target`) will overwrite the top-level organisation's primary name with the name of a target link inside a relationship!
  2. **Entity Expansion**: Target nodes reference ~9,050 additional organisation entities not declared as top-level records in `fullfile.zip`.
- **Resolution**:
  - `ods` SAX parsers strictly enforce `depth == 1 && !in_target` when scanning top-level entity properties.
  - `ods parquet` collects and resolves embedded target organisation nodes so that zero target references are orphaned.

---

### Gotcha 3: Tag Naming Variations (`<Rel>` vs `<Relationship>` & `<Succ>` vs `<Successor>`)
- **The Issue**: Element tag names vary between full names and abbreviations across `fullfile.zip`, `archive.zip`, and historical release versions:
  - `<Rel>` vs `<Relationship>`
  - `<Succ>` vs `<Successor>`
- **Resolution**: All XML parsing logic in `ods` uses pattern matching to accept both forms:
  ```rust
  b"Relationship" | b"Rel" => ...
  b"Successor" | b"Succ" => ...
  ```

---

### Gotcha 4: Missing `uniqueSuccId` in Archive ZIP Successors
- **The Issue**: Many `<Succ>` elements inside `archive.zip` omit explicit `uniqueSuccId` XML attributes.
- **Why it matters**: If a parser uses `uniqueSuccId` as a mandatory primary key for `successors.parquet`, ingestion will fail or drop historical successor links.
- **Resolution**: `ods` generates deterministic fallback primary keys (`succ_{index}`) whenever `uniqueSuccId` is missing from an XML element.

---

### Gotcha 5: Similar & Near-Identical Entity Names
- **The Issue**: Distinct organisations in NHS ODS XML often have near-identical names:
  - **`5HN16`**: `FAIRFIELD SURESTART CENTRE` *(No space)*
  - **`RY8NN`**: `FAIRFIELD SURE START CENTRE` *(With space)*
- **Resolution**: Never assume organisation name uniqueness. All join, indexing, and diffing operations in `ods` are keyed strictly by `ods_code`.

---

### Gotcha 6: Expected Non-100% Parent Linkage (Specialized GP Practices)
- **The Issue**: Active GP Practices link to ICB commissioners via relationship `RE4` (`is commissioned by`).
- **Why it matters**: Out of ~12,700 GP practice entries in TRUD XML, ~60 specialized practices (e.g. Armed Forces medical units, overseas practices, prison health services) do not have a parent ICB assigned in official NHS data.
- **Resolution**: Hierarchy completeness validation in `ods audit` enforces a **>95.0% linkage threshold** rather than 100.0%, correctly accounting for valid non-standard healthcare entities.

---

## 4. Summary Matrix of XML File Variants

| Feature / Artifact | `fullfile.zip` (`HSCOrgRefData_Full_*.xml`) | `archive.zip` (`HSCOrgRefData_Archive_*.xml`) |
| :--- | :--- | :--- |
| **Primary Scope** | Current & active/retired top-level entities | Closed legacy entities & historical successor maps |
| **Top-Level Orgs** | ~294,706 entities | Historical closed entities |
| **Target Node Expansion** | Adds ~9,050 target org references | Legacy target references |
| **Primary Parquet Output** | `orgs_all.parquet`, `orgs.parquet`, `roles.parquet`, `rels.parquet` | `successors.parquet` |

---

## 5. Structural Challenges & Data Shape Gotchas

### 1. Significant Roles Relegated to Secondary Roles (`RO261` vs `RO318`)
- **The Challenge**: In raw NHS TRUD XML, the 42 Statutory Integrated Care Boards (ICBs) are assigned primary role code `RO261` (`"strategic partnership"`), while their defining statutory role `RO318` (`"integrated care board"`) is relegated to a secondary `<Role>` element.
- **Impact**: Querying `SELECT * FROM orgs.parquet WHERE role = 'integrated care board'` returns 0 rows in `orgs.parquet` unless secondary role mappings or statutory name pattern matching are applied.

### 2. Regional Assignment Gaps & Unmapped Entities
- **The Challenge**: Out of ~294,000 total entities in `orgs_all.parquet`, over 200,000 entities (e.g., local clinic sites, independent sector providers, optical/dental practices) do not have a direct regional link assigned in the TRUD XML hierarchy.
- **Impact**: Any naive fallback (such as defaulting unmapped entities to London `Y56`) artificially inflates London entity counts to 200,000+. Unmapped entities must be grouped into an explicit `[UNMAPPED / OTHER]` category.

### 3. Reporting Sub-ICB Locations vs Statutory ICB Boards
- **The Challenge**: In TRUD XML, primary care practices are linked to Sub-ICB Locations (`RO319` / former CCG reporting codes - 213 distinct codes) rather than directly to the 42 Statutory Integrated Care Board bodies (`RO318`).
- **Impact**: Grouping raw `icb_code` values directly yields 213 sub-reporting codes (e.g., `NHS NORTH CENTRAL LONDON ICB - 93C`). Extracting the 42 Statutory ICBs requires mapping sub-ICB reporting codes back to their parent Statutory ICB entity.

---

For a complete breakdown of the organisational hierarchies across England, Scotland, Wales, Northern Ireland, and Crown Dependencies, see [docs/nhs.md](file:///Users/oli/Code/olizilla/ods/docs/nhs.md).
