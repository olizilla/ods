//! Integration tests enforcing ICB and Commissioner resolution invariants.

use ods::commands::ndjson;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_icb_resolution_prefers_active_targets_and_traverses_sub_icb() {
    let tmp = TempDir::new().unwrap();
    let xml_path = tmp.path().join("icb_resolution_test.xml");

    // Construct mock XML containing:
    // 1. QKK: Root ICB (RO318, Active)
    // 2. 72Q: Sub-ICB Location (RO319, Active), linked to QKK via RE5
    // 3. 08K: Closed CCG (RO98, Inactive)
    // 4. G85028: Active GP Practice, with RE4 to inactive 08K FIRST, and RE4 to active 72Q SECOND.
    // 5. LEGACY01: Legacy Practice, with ONLY RE4 to inactive 08K.
    let xml_content = r#"<?xml version="1.0" encoding="UTF-8"?>
<un:OrganisationManifest xmlns:un="http://refdata.hscic.gov.uk/org/v2-0-0">
  <un:ManifestHeader>
    <un:PublicationType value="Full" />
    <un:PublicationDate value="2026-07-31" />
    <un:PublicationSeqNum value="1" />
  </un:ManifestHeader>
  <un:Organisations>
    <un:Organisation orgRecordClass="RC1">
      <un:Name>NHS SOUTH EAST LONDON INTEGRATED CARE BOARD</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="QKK" />
      <un:Status value="Active" />
      <un:Roles>
        <un:Role id="RO318" uniqueRoleId="1" primaryRole="true">
          <un:Status value="Active" />
        </un:Role>
      </un:Roles>
    </un:Organisation>

    <un:Organisation orgRecordClass="RC1">
      <un:Name>NHS SOUTH EAST LONDON ICB - 72Q</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="72Q" />
      <un:Status value="Active" />
      <un:Roles>
        <un:Role id="RO319" uniqueRoleId="2" primaryRole="true">
          <un:Status value="Active" />
        </un:Role>
      </un:Roles>
      <un:Relationships>
        <un:Relationship id="RE5" uniqueRelId="10" status="Active">
          <un:Target>
            <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="QKK" />
          </un:Target>
        </un:Relationship>
      </un:Relationships>
    </un:Organisation>

    <un:Organisation orgRecordClass="RC1">
      <un:Name>NHS LAMBETH CCG</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="08K" />
      <un:Status value="Inactive" />
      <un:Roles>
        <un:Role id="RO98" uniqueRoleId="3" primaryRole="true">
          <un:Status value="Inactive" />
        </un:Role>
      </un:Roles>
      <un:Successors>
        <un:Successor uniqueSuccId="20">
          <un:Type value="Successor" />
          <un:Target>
            <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="QKK" />
          </un:Target>
        </un:Successor>
      </un:Successors>
    </un:Organisation>

    <un:Organisation orgRecordClass="RC1">
      <un:Name>STOCKWELL GROUP PRACTICE</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="G85028" />
      <un:Status value="Active" />
      <un:Roles>
        <un:Role id="RO177" uniqueRoleId="4" primaryRole="true">
          <un:Status value="Active" />
        </un:Role>
      </un:Roles>
      <un:Relationships>
        <un:Relationship id="RE4" uniqueRelId="30" status="Active">
          <un:Target>
            <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="08K" />
          </un:Target>
        </un:Relationship>
        <un:Relationship id="RE4" uniqueRelId="31" status="Active">
          <un:Target>
            <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="72Q" />
          </un:Target>
        </un:Relationship>
      </un:Relationships>
    </un:Organisation>

    <un:Organisation orgRecordClass="RC1">
      <un:Name>LEGACY PRACTICE</un:Name>
      <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="LEGACY01" />
      <un:Status value="Active" />
      <un:Roles>
        <un:Role id="RO177" uniqueRoleId="5" primaryRole="true">
          <un:Status value="Active" />
        </un:Role>
      </un:Roles>
      <un:Relationships>
        <un:Relationship id="RE4" uniqueRelId="40" status="Active">
          <un:Target>
            <un:OrgId root="2.16.840.1.113883.2.1.3.2.4.18.48" extension="08K" />
          </un:Target>
        </un:Relationship>
      </un:Relationships>
    </un:Organisation>
  </un:Organisations>
</un:OrganisationManifest>"#;

    fs::write(&xml_path, xml_content).unwrap();

    let (_prov, _concept_map, parsed) = ndjson::parse_single_pass(&xml_path).unwrap();
    let resolved = ndjson::resolve_hierarchies(parsed);

    // 1. Assert G85028 resolution
    let g85028 = resolved.get("G85028").expect("G85028 must be in resolved records");
    assert_eq!(
        g85028.commissioner_code.as_deref(),
        Some("72Q"),
        "G85028 commissioner_code must prefer active target 72Q over inactive 08K"
    );
    assert_eq!(
        g85028.commissioner.as_deref(),
        Some("NHS SOUTH EAST LONDON ICB - 72Q"),
        "G85028 commissioner name must match 72Q"
    );
    assert_eq!(
        g85028.icb_code.as_deref(),
        Some("QKK"),
        "G85028 icb_code must resolve to root ICB QKK"
    );
    assert_eq!(
        g85028.icb.as_deref(),
        Some("NHS SOUTH EAST LONDON INTEGRATED CARE BOARD"),
        "G85028 icb name must match root ICB QKK"
    );

    // 2. Assert LEGACY01 resolution via succession fallback
    let legacy = resolved.get("LEGACY01").expect("LEGACY01 must be in resolved records");
    assert_eq!(
        legacy.icb_code.as_deref(),
        Some("QKK"),
        "LEGACY01 icb_code must resolve to root ICB QKK via 08K's succession chain fallback"
    );
}
