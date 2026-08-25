# ODS NDJSON Schema Design

This document details the schema of the Newline Delimited JSON (NDJSON) intermediate file (`ods.ndjson`) produced by the `ods compile` command. It explains the source mapping of each field and differentiates between structures inherited from the source NHS XML schema and architectural design choices made for our own convenience.

## Sample NDJSON Record

Here is a typical compiled organisation record from `ods.ndjson`:

```json
{
  "ods_code": "D3H7C",
  "name": "THE MAPLES",
  "status": "Active",
  "role": "NHS TRUST SITE",
  "root": "2.16.840.1.113883.2.1.3.2.4.18.48",
  "assigning_authority_name": "HSCIC",
  "org_record_class": "RC2",
  "last_change_date": "2026-01-08",
  "dates": [
    {
      "type": "Operational",
      "start": "2026-01-08"
    }
  ],
  "geo_loc": {
    "location": {
      "address_lines": [
        "COUNTY HALL",
        "CROSS STREET"
      ],
      "town": "BEVERLEY",
      "postcode": "HU17 9BA",
      "country": "ENGLAND",
      "uprn": "100052020933"
    }
  },
  "roles": [
    {
      "id": "RO198",
      "display_name": "NHS TRUST SITE",
      "unique_role_id": "552136",
      "primary_role": true,
      "status": "Active",
      "dates": [
        {
          "type": "Operational",
          "start": "2026-01-08"
        }
      ]
    }
  ],
  "relationships": [
    {
      "id": "RE6",
      "display_name": "IS OPERATED BY",
      "unique_rel_id": "872263",
      "status": "Active",
      "dates": [
        {
          "type": "Operational",
          "start": "2026-01-08"
        }
      ],
      "target": {
        "ods_code": "RV9",
        "name": "HUMBER TEACHING NHS FOUNDATION TRUST",
        "root": "2.16.840.1.113883.2.1.3.2.4.18.48",
        "assigning_authority_name": "HSCIC",
        "primary_role_id": "RO197",
        "primary_role_display_name": "NHS TRUST",
        "primary_role_unique_role_id": "100941"
      }
    }
  ]
}
```

---

## Field-by-Field Breakdown

### 1. XML-Derived Fields (Inherited from Source Schema)
These fields directly mirror the structures in the source NHS TRUD XML schema (`HSCOrgRefData`). They are retained to preserve the full relational fidelity of the original data.

*   `root` & `assigning_authority_name`: Attributes of the `<OrgId>` element (`root="2.16.840..."`). In HL7/NHS specifications, these represent the unique registration authority namespaces.
*   `org_record_class`: Attribute of `<Organisation>` (e.g. `RC1` or `RC2`). This distinguishes base organizations from their geographical sites.
*   `last_change_date`: Attribute of `<Organisation>` tracking when any sub-element was modified in the source system.
*   `dates` (nested list): Nested `<Date>` elements tracking the historic operational/legal timeline of the organization.
*   `geo_loc` (nested object): Matches the `<GeoLoc><Location>` nested structure in XML. Contains `address_lines`, `town`, `postcode`, `country`, and `uprn`.
*   `roles` (nested list): Matches nested `<Role>` elements. It preserves the complete timeline of active and retired administrative functions held by the organisation.
*   `relationships` (nested list): Matches nested `<Rel>` elements mapping connections to target entities (like commissioning relationships).

---

### 2. Architectural Choices (Our Design Choices)
These fields are not present in the XML in this form. We introduced them to flatten the schema and optimize downstream Parquet queries:

*   **`role` (at root level)**: In the XML, there are no text role names; organisations only reference role IDs (like `RO198`). We look this up dynamically in the Concept Map parsed from `<CodeSystems>` at the top of the XML and denormalize the human-readable string directly to the root for ease of use.
*   **Renaming `post_code` to `postcode`**: Replaced the two-word database convention (`post_code`) with the standard UK spelling (`postcode`).
*   **Contacts Filtering & Normalisation**: ODS XML elements contain a flat list of `<Contact>` nodes. We extract `tel` and lower-case `http` (website) values, omitting deprecated contacts like `fax`.

---

## Alignment with Parquet Schema Goals

Now that we have designed the `orgs.parquet` target schema, we can look at what parts of the NDJSON intermediate record we could change to make the Parquet exporter even cleaner:

| Current NDJSON Shape | Parquet Goal | Design Choice Consideration |
| :--- | :--- | :--- |
| `"geo_loc": { "location": { ... } }` | `address`, `town`, `county`, `postcode` | The nested `geo_loc.location` structure is deeply nested and redundant. We could flatten this block directly to `address`, `town`, `county`, `postcode`, `country`, and `uprn` at the root of the JSON record. |
| `"dates": [ ... ]` | `start_date`, `end_date` | The raw `dates` array is rarely used. The flattened `start_date`/`end_date` at the root are sufficient. We could choose to drop the raw `dates` array from NDJSON. |
| `"relationships": [ ... ]` | Secondary `rels.parquet` table | Storing the entire nested `relationships` graph representation in every NDJSON row creates massive JSON files. We could choose to keep it in NDJSON as a source of truth for `rels.parquet` export, or write out relations separately. |
