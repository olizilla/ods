use ods::commands::info::{self, Args, OutputFormat};
use ods::ods_xml::{
    Location, OdsContact, OdsDate, OdsRecord, OdsRelationship, OdsRelationshipTarget, OdsRole,
    OdsSuccessor,
};
use ods::commands::parquet::{
    export_orgs, export_relationships, export_roles, export_successions,
};
use std::path::PathBuf;
use tempfile::TempDir;

#[allow(clippy::vec_init_then_push)]
fn setup_test_workspace() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("create temp dir");
    let dir = tmp.path().to_path_buf();

    let mut records = Vec::new();

    // 1. A82608 - GP Practice with outbound RE4 (is commissioned by 01K)
    records.push(OdsRecord {
        ods_code: "A82608".to_string(),
        name: "SEDBERGH MEDICAL PRACTICE".to_string(),
        status: "active".to_string(),
        role: "Prescribing Cost Centre".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: Some("2026-01-01".to_string()),
        dates: vec![OdsDate {
            date_type: "Operational".to_string(),
            start: Some("1974-04-01".to_string()),
            end: None,
        }],
        geo_loc: Some(Location {
            address_lines: vec![
                "SEDBERGH HEALTH CENTRE".to_string(),
                "LOFTUS HILL".to_string(),
            ],
            town: Some("SEDBERGH".to_string()),
            county: Some("CUMBRIA".to_string()),
            postcode: Some("LA10 5DL".to_string()),
            country: Some("ENGLAND".to_string()),
            uprn: Some("100052020933".to_string()),
        }),
        contacts: vec![
            OdsContact {
                contact_type: "tel".to_string(),
                value: "015396 20218".to_string(),
            },
            OdsContact {
                contact_type: "http".to_string(),
                value: "https://www.sedberghmp.nhs.uk".to_string(),
            },
        ],
        roles: vec![
            OdsRole {
                id: "RO177".to_string(),
                code: Some("RO177".to_string()),
                display_name: Some("Prescribing Cost Centre".to_string()),
                unique_role_id: "1001".to_string(),
                primary_role: true,
                status: "active".to_string(),
                dates: vec![OdsDate {
                    date_type: "Operational".to_string(),
                    start: Some("1974-04-01".to_string()),
                    end: None,
                }],
            },
            OdsRole {
                id: "RO76".to_string(),
                code: Some("RO76".to_string()),
                display_name: Some("GP Practice".to_string()),
                unique_role_id: "1002".to_string(),
                primary_role: false,
                status: "active".to_string(),
                dates: vec![],
            },
        ],
        relationships: vec![OdsRelationship {
            id: "RE4".to_string(),
            display_name: Some("IS COMMISSIONED BY".to_string()),
            unique_rel_id: "5001".to_string(),
            status: "active".to_string(),
            dates: vec![OdsDate {
                date_type: "Operational".to_string(),
                start: Some("2013-04-01".to_string()),
                end: None,
            }],
            target: OdsRelationshipTarget {
                ods_code: "01K".to_string(),
                name: Some("NHS MORECAMBE BAY CCG".to_string()),
                ..Default::default()
            },
        }],
        successors: vec![],
    });

    // 2. 01K - CCG
    records.push(OdsRecord {
        ods_code: "01K".to_string(),
        name: "NHS MORECAMBE BAY CCG".to_string(),
        status: "active".to_string(),
        role: "Clinical Commissioning Group".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: None,
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO98".to_string(),
            code: Some("RO98".to_string()),
            display_name: Some("Clinical Commissioning Group".to_string()),
            unique_role_id: "2001".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    // 3. A82608001 - Branch surgery with outbound RE6 (is operated by A82608)
    records.push(OdsRecord {
        ods_code: "A82608001".to_string(),
        name: "DR LUMB W & PARTNER".to_string(),
        status: "active".to_string(),
        role: "Branch Surgery".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "site".to_string(),
        last_change_date: None,
        dates: vec![OdsDate {
            date_type: "Operational".to_string(),
            start: Some("2010-01-01".to_string()),
            end: None,
        }],
        geo_loc: None,
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO180".to_string(),
            code: Some("RO180".to_string()),
            display_name: Some("Branch Surgery".to_string()),
            unique_role_id: "3001".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![OdsRelationship {
            id: "RE6".to_string(),
            display_name: Some("IS OPERATED BY".to_string()),
            unique_rel_id: "6001".to_string(),
            status: "active".to_string(),
            dates: vec![OdsDate {
                date_type: "Operational".to_string(),
                start: Some("2010-01-01".to_string()),
                end: None,
            }],
            target: OdsRelationshipTarget {
                ods_code: "A82608".to_string(),
                name: Some("SEDBERGH MEDICAL PRACTICE".to_string()),
                ..Default::default()
            },
        }],
        successors: vec![],
    });

    // 4. 0AF -> 0CE -> 0CY -> YDDTR succession chain
    records.push(OdsRecord {
        ods_code: "0AF".to_string(),
        name: "LEGACY ORG 0AF".to_string(),
        status: "inactive".to_string(),
        role: "Prescribing Cost Centre".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: None,
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO177".to_string(),
            code: Some("RO177".to_string()),
            display_name: Some("Prescribing Cost Centre".to_string()),
            unique_role_id: "4001".to_string(),
            primary_role: true,
            status: "inactive".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![OdsSuccessor {
            unique_succ_id: "succ1".to_string(),
            succ_type: "Successor".to_string(),
            dates: vec![OdsDate {
                date_type: "Legal".to_string(),
                start: Some("2012-10-01".to_string()),
                end: None,
            }],
            target: OdsRelationshipTarget {
                ods_code: "0CE".to_string(),
                name: Some("LEGACY ORG 0CE".to_string()),
                ..Default::default()
            },
        }],
    });

    records.push(OdsRecord {
        ods_code: "0CE".to_string(),
        name: "LEGACY ORG 0CE".to_string(),
        status: "inactive".to_string(),
        role: "Prescribing Cost Centre".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: None,
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO177".to_string(),
            code: Some("RO177".to_string()),
            display_name: Some("Prescribing Cost Centre".to_string()),
            unique_role_id: "4002".to_string(),
            primary_role: true,
            status: "inactive".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![OdsSuccessor {
            unique_succ_id: "succ2".to_string(),
            succ_type: "Successor".to_string(),
            dates: vec![OdsDate {
                date_type: "Legal".to_string(),
                start: Some("2014-10-01".to_string()),
                end: None,
            }],
            target: OdsRelationshipTarget {
                ods_code: "0CY".to_string(),
                name: Some("LEGACY ORG 0CY".to_string()),
                ..Default::default()
            },
        }],
    });

    records.push(OdsRecord {
        ods_code: "0CY".to_string(),
        name: "LEGACY ORG 0CY".to_string(),
        status: "inactive".to_string(),
        role: "Prescribing Cost Centre".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: None,
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO177".to_string(),
            code: Some("RO177".to_string()),
            display_name: Some("Prescribing Cost Centre".to_string()),
            unique_role_id: "4003".to_string(),
            primary_role: true,
            status: "inactive".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![OdsSuccessor {
            unique_succ_id: "succ3".to_string(),
            succ_type: "Successor".to_string(),
            dates: vec![OdsDate {
                date_type: "Legal".to_string(),
                start: Some("2016-04-01".to_string()),
                end: None,
            }],
            target: OdsRelationshipTarget {
                ods_code: "YDDTR".to_string(),
                name: Some("LEGACY ORG YDDTR".to_string()),
                ..Default::default()
            },
        }],
    });

    records.push(OdsRecord {
        ods_code: "YDDTR".to_string(),
        name: "LEGACY ORG YDDTR".to_string(),
        status: "active".to_string(),
        role: "Prescribing Cost Centre".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: None,
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO177".to_string(),
            code: Some("RO177".to_string()),
            display_name: Some("Prescribing Cost Centre".to_string()),
            unique_role_id: "4004".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    // 5. Trust RJZ with 12 active operated sites and 1 inactive operated site
    records.push(OdsRecord {
        ods_code: "RJZ".to_string(),
        name: "KING'S COLLEGE HOSPITAL NHS FOUNDATION TRUST".to_string(),
        status: "active".to_string(),
        role: "NHS Trust".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: None,
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO197".to_string(),
            code: Some("RO197".to_string()),
            display_name: Some("NHS Trust".to_string()),
            unique_role_id: "5000".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    for i in 1..=12 {
        let site_code = format!("RJZ{:02}", i);
        let site_name = format!("KINGS HOSPITAL SITE {:02}", i);
        records.push(OdsRecord {
            ods_code: site_code.clone(),
            name: site_name,
            status: "active".to_string(),
            role: "NHS Trust Site".to_string(),
            parent_organisation: None,
            region_code: None,
            root: None,
            assigning_authority_name: None,
            record_class: "site".to_string(),
            last_change_date: None,
            dates: vec![],
            geo_loc: None,
            contacts: vec![],
            roles: vec![OdsRole {
                id: "RO198".to_string(),
                code: Some("RO198".to_string()),
                display_name: Some("NHS Trust Site".to_string()),
                unique_role_id: format!("51{:02}", i),
                primary_role: true,
                status: "active".to_string(),
                dates: vec![],
            }],
            relationships: vec![OdsRelationship {
                id: "RE6".to_string(),
                display_name: Some("IS OPERATED BY".to_string()),
                unique_rel_id: format!("70{:02}", i),
                status: "active".to_string(),
                dates: vec![OdsDate {
                    date_type: "Operational".to_string(),
                    start: Some("2020-01-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "RJZ".to_string(),
                    name: Some("KING'S COLLEGE HOSPITAL NHS FOUNDATION TRUST".to_string()),
                    ..Default::default()
                },
            }],
            successors: vec![],
        });
    }

    // Inactive site RJZ99 with inactive RE6 to RJZ
    records.push(OdsRecord {
        ods_code: "RJZ99".to_string(),
        name: "CLOSED CLINIC 99".to_string(),
        status: "inactive".to_string(),
        role: "NHS Trust Site".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "site".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: None,
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO198".to_string(),
            code: Some("RO198".to_string()),
            display_name: Some("NHS Trust Site".to_string()),
            unique_role_id: "5199".to_string(),
            primary_role: true,
            status: "inactive".to_string(),
            dates: vec![],
        }],
        relationships: vec![OdsRelationship {
            id: "RE6".to_string(),
            display_name: Some("IS OPERATED BY".to_string()),
            unique_rel_id: "7099".to_string(),
            status: "inactive".to_string(),
            dates: vec![OdsDate {
                date_type: "Operational".to_string(),
                start: Some("2010-01-01".to_string()),
                end: Some("2020-01-01".to_string()),
            }],
            target: OdsRelationshipTarget {
                ods_code: "RJZ".to_string(),
                name: Some("KING'S COLLEGE HOSPITAL NHS FOUNDATION TRUST".to_string()),
                ..Default::default()
            },
        }],
        successors: vec![],
    });

    // 6. Isolated org with no relationships
    records.push(OdsRecord {
        ods_code: "ISOLATED".to_string(),
        name: "ISOLATED ORGANISATION".to_string(),
        status: "active".to_string(),
        role: "Prescribing Cost Centre".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: None,
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO177".to_string(),
            code: Some("RO177".to_string()),
            display_name: Some("Prescribing Cost Centre".to_string()),
            unique_role_id: "6000".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    let prov = ods::provenance::fixture_embedded("2026-07-31");

    let edges = ods::commands::parquet::build_succession_edges(&records);
    let (succ_closures, pred_closures) =
        ods::commands::parquet::compute_transitive_closures(&records, &edges);
    export_orgs(&dir, &records, &succ_closures, &pred_closures, Some(&prov), "2026-07-31").expect("export orgs");
    export_roles(&dir, &records, Some(&prov), "2026-07-31").expect("export roles");
    export_relationships(&dir, &records, Some(&prov), "2026-07-31").expect("export relationships");
    export_successions(&dir, &records, Some(&prov), "2026-07-31").expect("export successions");

    (tmp, dir)
}

#[test]
fn test_info_renders_full_detail_for_exact_code() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "A82608".to_string(),
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for A82608");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        output_str.contains("SEDBERGH MEDICAL PRACTICE"),
        "Expected name 'SEDBERGH MEDICAL PRACTICE', got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("A82608"),
        "Expected ODS Code A82608, got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("Class: org"),
        "Expected 'Class: org', got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("Status: active"),
        "Expected 'Status: active', got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("Prescribing Cost Centre") && output_str.contains("RO177"),
        "Expected Prescribing Cost Centre with RO177, got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("GP Practice") && output_str.contains("RO76"),
        "Expected GP Practice (RO76), got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("LA10 5DL"),
        "Expected postcode LA10 5DL, got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("https://www.sedberghmp.nhs.uk"),
        "Expected website in output, got:\n{}",
        output_str
    );
}

#[test]
fn test_info_json_format_and_succession_chain() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "0AF".to_string(),
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for 0AF with JSON");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    let json: serde_json::Value = serde_json::from_str(&output_str).expect("valid JSON output");

    assert_eq!(json["ods_code"], "0AF");
    assert_eq!(json["status"], "inactive");

    let succ = json["succession"]
        .as_array()
        .expect("succession must be array");
    assert_eq!(
        succ.len(),
        3,
        "Expected 3 succession hops for 0AF, got: {:?}",
        succ
    );

    assert_eq!(succ[0]["code"], "0CE");
    assert_eq!(succ[0]["depth"], 1);
    assert_eq!(succ[0]["date"], "2012-10-01");

    assert_eq!(succ[1]["code"], "0CY");
    assert_eq!(succ[1]["depth"], 2);
    assert_eq!(succ[1]["date"], "2014-10-01");

    assert_eq!(succ[2]["code"], "YDDTR");
    assert_eq!(succ[2]["depth"], 3);
    assert_eq!(succ[2]["date"], "2016-04-01");
}

#[test]
fn test_info_case_insensitive() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "a82608".to_string(),
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for lowercase a82608");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        output_str.contains("A82608"),
        "Expected A82608 in output, got:\n{}",
        output_str
    );
}

#[test]
fn test_info_nonexistent_code_fails() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    let result = info::run_with_writer(
        Args {
            ods_code: "NONEXISTENT999".to_string(),
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    );

    assert!(result.is_err(), "Expected error for nonexistent ODS code");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("NONEXISTENT999"),
        "Expected error message to contain 'NONEXISTENT999', got: {}",
        err_msg
    );
}

#[test]
fn test_info_relationships_section_shows_outbound() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "A82608".to_string(),
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for A82608");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        output_str.contains("Relationships (1 active, 0 inactive)"),
        "Expected 'Relationships (1 active, 0 inactive)' section, got:\n{}",
        output_str
    );
    // Outbound commissioning link
    assert!(
        output_str.contains("commissioned by"),
        "Expected 'commissioned by' relationship group, got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("01K") && output_str.contains("NHS MORECAMBE BAY CCG"),
        "Expected commissioner code 01K and name in relationships, got:\n{}",
        output_str
    );
    // Inbound links are absent from table
    assert!(
        !output_str.contains("A82608001"),
        "Inbound links must not appear in info table"
    );
}

#[test]
fn test_info_relationships_in_json() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "A82608".to_string(),
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for A82608 in JSON");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    let json: serde_json::Value = serde_json::from_str(&output_str).expect("valid JSON");

    let rels = json["relationships"]
        .as_array()
        .expect("relationships must be an array");
    assert!(!rels.is_empty(), "A82608 must have relationships");

    let has_comm = rels
        .iter()
        .any(|r| r["code"] == "01K" && r["direction"] == "outbound" && r["rel_code"] == "RE4");
    assert!(
        has_comm,
        "A82608 must have outbound RE4 relationship to 01K"
    );

    let has_inbound_site = rels
        .iter()
        .any(|r| r["code"] == "A82608001" && r["direction"] == "inbound" && r["rel_code"] == "RE6");
    assert!(
        has_inbound_site,
        "A82608 must have inbound RE6 relationship from A82608001"
    );
}

#[test]
fn test_info_trust_inbound_relationships_not_in_card() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "RJZ".to_string(),
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for RJZ in Table");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        !output_str.contains("Relationships"),
        "RJZ has no outbound relationships, so Relationships section must be omitted, got:\n{}",
        output_str
    );
}

#[test]
fn test_info_trust_relationships_in_json() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "RJZ".to_string(),
            format: OutputFormat::Json,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for RJZ in JSON");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    let json: serde_json::Value = serde_json::from_str(&output_str).expect("valid JSON");

    let rels = json["relationships"]
        .as_array()
        .expect("relationships must be an array");
    // All 13 inbound relationships present in JSON without truncation
    let inbound_re6: Vec<_> = rels
        .iter()
        .filter(|r| r["rel_code"] == "RE6" && r["direction"] == "inbound")
        .collect();
    assert_eq!(
        inbound_re6.len(),
        13,
        "Expected 13 inbound RE6 relationships in JSON, got: {}",
        inbound_re6.len()
    );

    assert!(inbound_re6
        .iter()
        .any(|r| r["code"] == "RJZ01" && r["status"] == "active"));
    assert!(inbound_re6
        .iter()
        .any(|r| r["code"] == "RJZ99" && r["status"] == "inactive"));
}

#[test]
fn test_info_leaf_operates_under_parent() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "A82608001".to_string(),
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for A82608001");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        output_str.contains("Relationships (1 active, 0 inactive)"),
        "Leaf entity A82608001 must have relationships heading, got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("operated by"),
        "Leaf entity must show outbound 'operated by', got:\n{}",
        output_str
    );
    assert!(
        output_str.contains("A82608") && output_str.contains("SEDBERGH MEDICAL PRACTICE"),
        "Leaf entity must name parent A82608, got:\n{}",
        output_str
    );
}

#[test]
fn test_info_entity_without_relationships_omits_section() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "ISOLATED".to_string(),
            format: OutputFormat::Table,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for ISOLATED");

    let output_str = String::from_utf8(out).expect("valid UTF-8");
    assert!(
        !output_str.contains("Relationships"),
        "ISOLATED has no relationships, so Relationships section must be omitted, got:\n{}",
        output_str
    );
}



#[test]
fn test_info_markdown_format_structure_and_no_ansi() {
    let (_tmp, parquet_dir) = setup_test_workspace();

    let mut out = Vec::new();
    info::run_with_writer(
        Args {
            ods_code: "A82608".to_string(),
            format: OutputFormat::Markdown,
            input: Some(parquet_dir.clone()),
            ..Default::default()
        },
        &mut out,
        &parquet_dir,
    )
    .expect("info::run_with_writer should succeed for A82608 in Markdown");

    let s = String::from_utf8(out).expect("valid UTF-8");

    // 1. Pipe table structure
    assert!(s.contains("| ODS Code"), "Must contain markdown pipe table for fields");
    assert!(s.contains("| Role"), "Must contain markdown pipe table for roles");
    assert!(s.contains("| Relationship"), "Must contain markdown pipe table for relationships");

    // 2. Section headings start with ## and are followed by a blank line
    assert!(s.contains("## Roles (2 active, 0 inactive)\n\n|"), "Roles heading must be ## followed by blank line");
    assert!(s.contains("## Relationships (1 active, 0 inactive)\n\n|"), "Relationships heading must be ## followed by blank line");

    // 3. Start and End columns present in markdown tables
    assert!(s.contains("| Start"), "Roles/relationships table must have Start column");
    assert!(s.contains("| End"), "Roles/relationships table must have End column");

    // 4. No ANSI escape codes
    assert!(!s.contains("\x1b"), "Markdown output must not contain ANSI escape codes");
}

#[test]
fn test_info_default_format_is_table() {
    assert_eq!(Args::default().format, OutputFormat::Table);
}








