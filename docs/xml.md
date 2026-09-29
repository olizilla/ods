# NHS TRUD ODS XML structure and gotchas

This document details the internal structure, quirks, quality edge-cases, and parser gotchas of the official NHS Organisation Data Service (ODS) XML data distributions downloaded from TRUD (Technology Reference data Update Distribution).

## The archive product

A TRUD release holds two XML files, in two zips: `fullfile.zip` is what NHS England keeps in its live product, and `archive.zip` is what it has moved out. NHS England
[documents the rule](https://www.odsdatasearchandexport.nhs.uk/referenceDataCatalogue/ODS-Archived-or-Deleted-Records_592550682.html)
this way: "Everything with an operational close date that falls before the most recently defined cut-off is published in the archive product, instead of live data", currently "on or before 31 March 2017".

- **The cut-off moves.** In April 2023 it went from 2012-03-31 to 2017-03-31, taking about 34,000 organisations from the full file to the archive in one release. A series built from the full file alone shows them vanishing in a month when they closed years earlier. The cut-off will move again.
- **`refOnly` stubs.** A record that live data still references stays in the full file as a skeleton, `<Organisation refOnly="true">`, with its relationships and other content omitted. The archive does the same for live organisations that archived ones reference. In the 2026-08-28 release the full file holds 10,249 stubs and the archive 11,740, and every stub has its complete record in the other file.
- **The two files in one zip.** On 2026-08-28 the full file holds 306,201 organisations (660 MB of XML) and the archive 86,705 (174 MB), each declaring its own `<RecordCount>` and matching it exactly. Together that is 392,906 records, 21,989 of them stubs, so 370,917 organisations.
- **Deletion is separate, and rare.** Being archived doesn't mean being deleted, and archiving isn't how organisations leave the register: between the May and August 2026 releases 165 codes disappeared from the full file and none appeared in the archive.

`ods` reads both files and merges them: where a code is in both, the complete record wins and the stub is dropped, and a code with two stubs fails the build. So the dataset doesn't inherit the split, and `orgs.parquet` needs no column saying which file a row came from. [parquet.md](./parquet.md) describes the result.

## Distribution and archive packaging

The NHS ODS data is published monthly on TRUD as a single root `.zip` archive (e.g., `hscorgrefdataxml_data_5.0.0_YYYYMMDD000001.zip`).

Inside this ZIP container are two zipped XML files:

1. **`fullfile.zip`** (`HSCOrgRefData_Full_*.xml`):
   - NHS England's live product: the active organisations and those that closed after the archive cut-off, with their role assignments and relationships.
   - Read by `ods make` together with `archive.zip`, and merged into `orgs.parquet`.

2. **`archive.zip`** (`HSCOrgRefData_Archive_*.xml`):
   - Everything with an operational close date on or before the published cut-off (currently 31 March 2017): organisations closed decades ago, with their roles, relationships and successions.
   - Read by `ods make` alongside `fullfile.zip`. See [nhs.md](./nhs.md#the-archive-product).
   - Merged with `fullfile.zip`'s successions into `successions.parquet` — see [parquet.md](./parquet.md#successionsparquet) for its row count.

The two files have the same shape. Each `<Manifest>` declares schema `Version` `2-0-0` and
`PublicationType` `Full`. They differ in `PublicationSeqNum` (4748 and 4749 on 2026-08-28), in
`ContentDescription` (`HSCOrgRefData_Full_20260827`, `HSCOrgRefData_Archive_20260827`), and in
`RecordCount`. Neither is a delta: each release is complete on its own.

The split follows NHS England's cut-off date, and the cut-off moves. [nhs.md](./nhs.md#the-archive-product)
has the rule, the `refOnly` stubs that keep each file self-consistent, and what moving the
cut-off did in April 2023. One more fact from the 2026-08-28 release: an organisation found only
in the archive is the target of no active relationship.

## Core XML element hierarchy

The XML root element `<MANIFEST>` contains publication metadata (`PublicationDate`), followed by a flat stream of `<Organisation>` elements:

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

## Structural quirks and parser gotchas

During the development of `ods`, we identified several non-obvious structural quirks in NHS TRUD XML files. Any parser or auditing tool built for NHS ODS XML needs to account for these behaviours:

### Missing top-level `<Status>` tags (NHS Region offices)
Active NHS administrative bodies (such as NHS Region offices `Y58`, `Y59`, `Y60`, etc.) omit top-level `<Status value="Active">` elements in raw TRUD XML. A parser checking only the top-level `<Status>` tag will treat active NHS Region offices as inactive, dropping them from active analytical queries (`orgs.parquet`). When the top-level `<Status>` is missing, `ods` falls back to the primary role's status: if `primary_role.status == "Active"`, the entity is treated as active.

### Tag naming variations (`<Rel>` vs `<Relationship>`, `<Succ>` vs `<Successor>`)
Element tag names vary between full names and abbreviations across `fullfile.zip`, `archive.zip`, and historical release versions: `<Rel>` vs `<Relationship>`, `<Succ>` vs `<Successor>`. All XML parsing logic in `ods` uses pattern matching to accept both forms:
```rust
b"Relationship" | b"Rel" => ...
b"Successor" | b"Succ" => ...
```

### Expected non-100% commissioning linkage (specialised GP practices)
Active GP Practices link to commissioners via relationship `RE4` (`is commissioned by`). Out of ~12,700 GP practice entries in TRUD XML, ~60 specialised practices (e.g. Armed Forces medical units, overseas practices, prison health services) don't have a commissioning link assigned in official NHS data. Rather than synthesising or guessing missing parent links into flattened hierarchy columns, `ods` publishes the raw relationships in `relationships.parquet` as stated by NHS TRUD.

## Summary of XML file variants

| Feature / artefact | `fullfile.zip` (`HSCOrgRefData_Full_*.xml`) | `archive.zip` (`HSCOrgRefData_Archive_*.xml`) |
| :--- | :--- | :--- |
| **Primary scope** | Current & active/retired top-level entities | Closed legacy entities & historical successor maps |
| **Organisations** (2026-08-28) | 306,201, of which 10,249 are `refOnly` stubs | 86,705, of which 11,740 are `refOnly` stubs |
| **Parquet output** | Both files merge into `orgs.parquet`, `roles.parquet`, `relationships.parquet` and `successions.parquet` | (same tables) |

## Structural challenges and data shape gotchas

### Significant roles relegated to secondary roles (`RO261` vs `RO318`)
In raw NHS TRUD XML, the 42 Statutory Integrated Care Boards (ICBs) are assigned primary role code `RO261` (`"strategic partnership"`), while their defining statutory role `RO318` (`"integrated care board"`) is relegated to a secondary `<Role>` element. Querying on primary role code alone misses ICBs — use `list_contains(role_codes, 'RO318')` or `list_contains(role_names, 'Integrated Care Board')` on `orgs.parquet`.

### Regional assignment gaps and unmapped entities
Out of ~371,000 total entities in `orgs.parquet`, over 200,000 (e.g., local clinic sites, independent sector providers, optical/dental practices) don't have a direct regional link (`RE5`) assigned in the TRUD XML hierarchy. `RE5 IS LOCATED IN THE GEOGRAPHY OF` is marked legacy by NHS ODS and only links a subset of entities.

### Reporting sub-ICB locations vs statutory ICB boards
In TRUD XML, primary care practices are linked via `RE4` to Sub-ICB Locations (`RO319` / former CCG reporting codes — 213 distinct codes) rather than directly to the 42 Statutory Integrated Care Board bodies (`RO318`). Grouping raw `RE4` target codes yields 213 sub-reporting codes (e.g., `NHS NORTH CENTRAL LONDON ICB - 93C`). Finding the Statutory ICBs requires traversing relationships in `relationships.parquet`.
