# UK Health Systems & Organisation Structures

This document defines the organisational hierarchies and structures of the health systems across the United Kingdom and Crown Dependencies as revealed by the NHS Organisation Data Service (ODS) TRUD datasets.

---

## Overview: Sovereign UK Health Systems

The NHS Organisation Data Service (ODS) dataset is not exclusive to England; it encompasses all four sovereign health systems of the United Kingdom, alongside the Crown Dependencies. Each jurisdiction operates its own distinct administrative and operational hierarchy:

| Jurisdiction | Primary Governing Bodies | Primary Practice / Site Count | Key Role Codes |
| :--- | :--- | :--- | :--- |
| **England** | 7 NHS England Regions & 42 Statutory Integrated Care Boards (ICBs) | ~13,000 Practices / ~38,000 Sites | `RO209`, `RO318`, `RO197`, `RO177` |
| **Scotland** | 14 Territorial NHS Scottish Health Boards | 1,242 Scottish GP Practices | `RO190` |
| **Wales** | 7 Local Health Boards (Bddau Iechyd Lleol) | 7 Welsh Regions & LHB Sites | `RO272`, `RO328` |
| **Northern Ireland** | 5 Health and Social Care (HSC) Trusts & LCGs | 321 NI GP Practices | `RO197` (HSC Trust), `RO190` |
| **Isle of Man** | Manx Care & Health Directorates | 30 Practices & Manx Facilities | `RO138`, `RO140` |
| **Channel Islands** | States of Jersey, Guernsey & Alderney Health Services | 46 Medical Sites & Providers | `RO126` |

---

## NHS England Hierarchy Architecture

In England, health service delivery is structured under the Health and Care Act 2022 into 4 distinct operational hierarchy archetypes that cover over 95% of active organisations:

### Archetype 1: Primary Care Architecture (GP Surgeries, PCNs & ICBs) — *~13,000 Practices*
```text
NHS England Region (RO209 — e.g. Y56 London Region)
  └── Integrated Care Board (RO318 / RO261 — e.g. QMJ NHS North Central London ICB)
        ├── Primary Care Network (RO313 — e.g. U59980 Western Dales PCN) [RE8 partner link]
        │     └── Main GP Practice (RO177 prescribing cost centre — e.g. Hampstead Group Practice)
        │           └── Branch Surgery (RO178 site) [RE6 operated by link]
        └── Community Pharmacy & Dental Practice (RO181 / RO110) [RE4 commissioned by link]
```

### Archetype 2: Secondary & Tertiary Care Architecture (Hospitals & NHS Trusts) — *~38,000 Hospital Sites*
```text
NHS England Region (RO209 — e.g. Y56 London Region)
  └── NHS Foundation Trust / Acute Trust (RO197 — e.g. RAL Royal Free London NHS FT)
        ├── Main Acute Hospital (RO198 site — e.g. Royal Free Hospital) [RE6 operated by link]
        ├── Community Hospital (RO198 site — e.g. Chase Farm Hospital)
        ├── Diagnostic Centre (RO198 site — e.g. Finchley Memorial CDC)
        └── Health Centre / Outpatient Clinic (RO198 site)
```

### Archetype 3: Commercial Healthcare Architecture (Retail Pharmacies, Opticians, Private Hospitals) — *~35,000 Sites*
```text
Commercial Provider HQ (RO157 / RO104 — e.g. Boots UK Limited / Specsavers)
  └── High Street Branch / Private Clinic Site (RO181 / RO176) [RE6 operated by link]
        └── Commissioning Link ──► Local Integrated Care Board (ICB) [RE4 commissioned by link]
```

### Archetype 4: Public Health & Social Care Architecture (Care Homes & Schools) — *~55,000 Sites*
```text
Local Authority / County Council (RO141 — e.g. Camden Council)
  ├── Social Care Provider (RO104) ──► Care Home / Nursing Site (RO101) [RE6 operated by link]
  └── School / Academy Trust (RO221) ──► Primary & Secondary School Site [RE5 geography link]
```

### Key Relationship Edges in England:
- **`RE4` (`is commissioned by`)**: 148,000+ links connecting GP practices, PCNs, pharmacies, and dentists to their Statutory ICB.
- **`RE6` (`is operated by`)**: 202,000+ links connecting hospital sites, clinics, care units, and commercial branches to their parent Trust, HQ, or Practice.
- **`RE8` (`is partner to`)**: 8,300+ links connecting GP practices into Primary Care Networks (PCNs).
- **`RE5` (`is located in the geography of`)**: 292,000+ links connecting entities to Government Office Regions, Local Authorities, or Local Health Districts.

---

## NHS Scotland (NHS Alba) Hierarchy Architecture

In Scotland, healthcare delivery is unified under 14 territorial NHS Scottish Health Boards (`RO190`) that integrate primary, secondary, and community care within defined geographic regions:

```text
NHS Scotland (National Strategic Level)
  └── 14 Territorial Scottish Health Boards (RO190)
        ├── NHS Lothian (SL9)
        ├── NHS Greater Glasgow and Clyde (SG9)
        ├── NHS Highland (SH9)
        ├── NHS Grampian (SN9)
        ├── NHS Tayside (ST9)
        ├── NHS Fife (SF9)
        ├── NHS Ayrshire and Arran (SA9)
        ├── NHS Borders (SB9)
        ├── NHS Dumfries and Galloway (SY9)
        ├── NHS Forth Valley (SV9)
        ├── NHS Lanarkshire (SL9)
        ├── NHS Orkney (OR9)
        ├── NHS Shetland (SH9)
        └── NHS Western Isles (WI9)
              ├── Scottish GP Practices (1,242 active prescribing centres)
              ├── Community Hospitals & Acute NHS Facilities
              └── Scottish Optical & Dental Services
```

### Key Characteristics in Scotland:
- **Single-Tier Territorial Integration**: Unlike England's split between ICBs and NHS Trusts, Scottish Health Boards directly manage both GP primary care commissioning and hospital operations within their region.

---

## NHS Wales (GIG Cymru) Hierarchy Architecture

In Wales, healthcare is delivered through 7 integrated Local Health Boards (`RO272`) and 3 NHS Trusts, organized geographically across 7 Welsh Regions (`RO328`):

```text
Welsh Government Health & Social Services Group
  └── 7 Local Health Boards (RO272 / Bwrdd Iechyd Lleol)
        ├── Aneurin Bevan University Health Board (7A2)
        ├── Betsi Cadwaladr University Health Board (7A1)
        ├── Cardiff and Vale University Health Board (7A4)
        ├── Cwm Taf Morgannwg University Health Board (7A5)
        ├── Hywel Dda University Health Board (7A3)
        ├── Powys Teaching Health Board (7A6)
        └── Swansea Bay University Health Board (7A7)
              ├── Welsh Primary Care & GP Practices
              ├── Welsh Acute Hospitals & Community Health Centres
              └── Specialized Welsh NHS Trusts (Velindre NHS Trust, Welsh Ambulance Services)
```

### Key Characteristics in Wales:
- **Unified Health Boards**: Local Health Boards in Wales own and operate hospital facilities while directly commissioning primary care services for their resident populations.

---

## Health and Social Care Northern Ireland (HSC NI) Hierarchy Architecture

In Northern Ireland, health and social care services are integrated under the Department of Health NI and delivered via 5 regional Health and Social Care (HSC) Trusts:

```text
Department of Health Northern Ireland (DoH NI)
  └── Health and Social Care Board (HSCB) / Strategic Planning and Performance Group
        └── 5 HSC Trusts (RO197 / RO190)
              ├── Belfast Health and Social Care Trust
              ├── Northern Health and Social Care Trust
              ├── South Eastern Health and Social Care Trust
              ├── Southern Health and Social Care Trust
              └── Western Health and Social Care Trust
                    └── 5 Local Commissioning Groups (LCGs)
                          └── Northern Ireland GP Practices (321 active prescribing cost centres)
```

### Key Characteristics in Northern Ireland:
- **Health + Social Care Integration**: HSC NI structurally merges social care services and healthcare delivery within the same HSC Trusts.

---

## Crown Dependencies (Isle of Man & Channel Islands)

The Crown Dependencies maintain autonomous health administrations represented in ODS data:

### Isle of Man (Manx Care)
- **Governing Body**: `YJ1` *Isle of Man Statutory Board Manx Care* (`RO138`) & `YAC` *Department of Health*.
- **Facilities**: Nobles Hospital (`YK301`), Ramsey Cottage Hospital (`YK101`), 30 GP prescribing centres, 36 pharmacies, and 20 dental practices.

### Channel Islands (States of Jersey, Guernsey & Alderney)
- **Governing Bodies**: `YAD` *States of Jersey Health & Social Services*, `YAE` *States of Guernsey Health & Social Services*, `YAF` *States of Alderney*.
- **Facilities**: 46 medical prescribing centres, pathology facilities, and independent sector healthcare providers.

---

## Specialized & Cross-Jurisdictional Facilities

Across all jurisdictions, a set of specialized healthcare settings and infrastructure nodes exist outside standard regional commissioning models:

1. **Custodial & Secure Healthcare Settings**:
   - **Police Custody Suites** (357 sites), **Courts** (96 sites), and **Prisons** (96 sites): Facilities where primary care providers deliver healthcare under secure commissioning contracts.
   - **Sexual Assault Referral Centres (SARCs)** (49 sites): Emergency multi-agency healthcare facilities.
2. **National Diagnostic & Clinical Networks**:
   - **Pathology Labs** (267 labs): Shared diagnostic laboratories serving multiple acute trusts.
   - **Appliance Contractors** (116 suppliers): Medical device suppliers (stoma, orthotics) directly registered for prescription payment.
3. **National Infrastructure & Executive Agencies**:
   - **Application Service Providers** (366) & **NHS Support Agencies** (151): IT infrastructure points (Spine, e-Prescribing hubs).
   - **Executive Agencies & Special Health Authorities** (174): Arms-length bodies such as *UK Health Security Agency (UKHSA)*, *NHS Blood and Transplant (NHSBT)*, and *NHS Resolution*.

---

## Regional Resolution Coverage & Unmapped Entity Breakdown

Through multi-hop transitive relationship resolution (`GP Practice ➔ Sub-ICB ➔ Statutory ICB ➔ NHS Region`), `ods` resolves regional assignments for **85.0% of all active entities** (183,864 orgs out of 216,886).

### Resolved Categories (98%+ Coverage):
- **GP Practices (`prescribing cost centre`)**: 98.7% resolved (12,672 out of 12,841 practices).
- **General Dental Practices**: 98.9% resolved (9,683 out of 9,789 clinics).
- **Community Pharmacies**: 93.5% resolved (10,454 out of 11,177 branches).
- **Optical Clinics**: 98.3% resolved (6,795 out of 6,910 sites).
- **Acute Trust Hospitals**: 95.4% resolved (36,470 out of 38,254 sites).

### Unmapped Categories (The Remaining 15.0%):
The 33,022 active entities without an NHS England Region link represent non-commissioned private providers, corporate headquarters, or non-English entities:
1. **Private CQC Social Care Providers** (18,681 orgs): Independent residential care home providers registered with CQC without an NHS commissioning link.
2. **Corporate Headquarters** (8,262 orgs): Corporate headquarters of pharmacy companies (4,872 HQs) and optical chains (3,390 HQs).
3. **IT Infrastructure & National Agencies** (517 sites): Application Service Providers, Spine nodes, and Special Health Authorities.
4. **Devolved Administrations** (305 orgs): Northern Ireland GP practices managed under HSC NI.

## The archive product

A TRUD release holds two XML files, in two zips: `fullfile.zip` is what NHS England keeps in its live product, and `archive.zip` is what it has moved out. NHS England
[documents the rule](https://www.odsdatasearchandexport.nhs.uk/referenceDataCatalogue/ODS-Archived-or-Deleted-Records_592550682.html)
this way: "Everything with an operational close date that falls before the most recently defined cut-off is published in the archive product, instead of live data", currently "on or before 31 March 2017".

- **The cut-off moves.** In April 2023 it went from 2012-03-31 to 2017-03-31, taking about 34,000 organisations from the full file to the archive in one release. A series built from the full file alone shows them vanishing in a month when they closed years earlier. The cut-off will move again.
- **`refOnly` stubs.** A record that live data still references stays in the full file as a skeleton, `<Organisation refOnly="true">`, with its relationships and other content omitted. The archive does the same for live organisations that archived ones reference. In the 2026-08-28 release the full file holds 10,249 stubs and the archive 11,740, and every stub has its complete record in the other file.
- **The two files in one zip.** On 2026-08-28 the full file holds 306,201 organisations (660 MB of XML) and the archive 86,705 (174 MB), each declaring its own `<RecordCount>` and matching it exactly. Together that is 392,906 records, 21,989 of them stubs, so 370,917 organisations.
- **Deletion is separate, and rare.** Being archived doesn't mean being deleted, and archiving isn't how organisations leave the register: between the May and August 2026 releases 165 codes disappeared from the full file and none appeared in the archive.

`ods` reads both files and merges them: where a code is in both, the complete record wins and the stub is dropped, and a code with two stubs fails the build. So the dataset doesn't inherit the split, and `orgs.parquet` needs no column saying which file a row came from. [parquet.md](./parquet.md) describes the result.

### A code published twice

Very rarely a code is a complete record twice, not a record and a stub. Three of the 96 TRUD releases from 2018-06-29 to 2026-08-28 show it, in two different ways:

- **One organisation, re-recorded for a role change.** `T1201`, NHS LOGISTICS AUTHORITY, is in the archive of the 2023-03-31 and 2023-04-28 releases twice, with the same name, address and telephone: 2000-04-01 to 2003-03-31 holding `RO169`, then 2003-04-01 to 2004-09-30 holding `RO189`. A succession links `T1201` to itself.
- **A code reused for a different organisation.** `FEF03` in the 2025-05-30 release is BARKING, HAVERING & BRENTWOOD DISTRICT LAUNDRY COMMON SERVICE AGENCY in the archive (closed 1993-03-31) and MCGILLS DONCASTER, a pharmacy opened 2025-04-23, in the full file. Nothing in either file references `FEF03`.

NHS England has since resolved both the same way. In the 2026-08-28 release `T1201` appears once, for the later period, and `FEF03` once, as MCGILLS DONCASTER: the laundry record has gone from the archive.

`ods` applies the same rule when it builds an earlier release. The full file's record wins over the archive's, and within one file the record whose operational period starts later wins. The dropped record is named on stderr at build time:

```text
! FEF03 has a complete record in both files: kept the full file's, dropped the archive's (1991-04-01 to 1993-03-31)
! T1201 has two complete records in HSCOrgRefData_Archive_20230328.xml: kept 2003-04-01 to 2004-09-30, dropped 2000-04-01 to 2003-03-31
```

Two complete records in one file with no operational start on either, or with the same start, fail the build: the source hasn't said which is later, and `ods` doesn't guess. Two stubs for one code fail too.
