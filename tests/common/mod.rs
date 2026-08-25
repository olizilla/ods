use ods::commands::ndjson::{
    Location, OdsContact, OdsDate, OdsRecord, OdsRelationship, OdsRelationshipTarget, OdsRole,
    OdsSuccessor,
};
use ods::commands::parquet::{
    build_succession_edges, compute_transitive_closures, export_orgs, export_orgs_all,
    export_relationships, export_roles, export_successions,
};
use ods::provenance::OdsProvenance;
use std::path::PathBuf;
use tempfile::TempDir;

#[allow(clippy::vec_init_then_push)]
pub fn setup_find_test_workspace() -> (TempDir, PathBuf) {
    let tmp = TempDir::new().expect("create temp dir");
    let dir = tmp.path().to_path_buf();

    let mut records = Vec::new();

    // 1. A82608 - SEDBERGH MEDICAL PRACTICE (GP Practice with RO177 and RO76)
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
                dates: vec![],
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
            dates: vec![],
            target: OdsRelationshipTarget {
                ods_code: "01K".to_string(),
                name: Some("NHS MORECAMBE BAY CCG".to_string()),
                ..Default::default()
            },
        }],
        successors: vec![],
    });

    // 2. A82608001 - DR LUMB W & PARTNER (Branch Surgery)
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
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["LOFTUS HILL".to_string()],
            town: Some("SEDBERGH".to_string()),
            county: Some("CUMBRIA".to_string()),
            postcode: Some("LA10 5DL".to_string()),
            country: Some("ENGLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO180".to_string(),
            code: Some("RO180".to_string()),
            display_name: Some("Branch Surgery".to_string()),
            unique_role_id: "1003".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![OdsRelationship {
            id: "RE6".to_string(),
            display_name: Some("IS OPERATED BY".to_string()),
            unique_rel_id: "5002".to_string(),
            status: "active".to_string(),
            dates: vec![],
            target: OdsRelationshipTarget {
                ods_code: "A82608".to_string(),
                name: Some("SEDBERGH MEDICAL PRACTICE".to_string()),
                ..Default::default()
            },
        }],
        successors: vec![],
    });

    // 3. The other 12 Sedbergh active records to make exactly 14 in Sedbergh
    let other_sedbergh = vec![
        ("8GJ58", "SEDBERGH HEALTH CENTRE PHARMACY", "site", "RO182", "Pharmacy"),
        ("D2E8H", "SEDBERGH FIRST RESPONDERS", "org", "RO76", "GP Practice"),
        ("EE112233", "SEDBERGH OPTOMETRY", "site", "RO182", "Pharmacy"),
        ("EE112331", "SEDBERGH CARE HOME", "site", "RO182", "Pharmacy"),
        ("EE112451", "SEDBERGH DENTAL CARE", "site", "RO110", "General Dental Practice"),
        ("EE137269", "SEDBERGH COMMUNITY CLINIC", "site", "RO182", "Pharmacy"),
        ("FLG02", "SEDBERGH AMBULANCE STATION", "site", "RO182", "Pharmacy"),
        ("RNN88", "SEDBERGH NURSING HOME", "site", "RO182", "Pharmacy"),
        ("RW5OX", "SEDBERGH PHYSIOTHERAPY", "site", "RO182", "Pharmacy"),
        ("RX796", "SEDBERGH MEDICAL SERVICES", "org", "RO76", "GP Practice"),
        ("V25604", "MAIN STREET DENTAL SURGERY", "site", "RO110", "General Dental Practice"),
        ("VN6C2", "SEDBERGH REHABILITATION CENTRE", "site", "RO182", "Pharmacy"),
    ];

    for (code, name, rclass, role_code, role_name) in other_sedbergh {
        records.push(OdsRecord {
            ods_code: code.to_string(),
            name: name.to_string(),
            status: "active".to_string(),
            role: role_name.to_string(),
            parent_organisation: None,
            region_code: None,
            root: None,
            assigning_authority_name: None,
            record_class: rclass.to_string(),
            last_change_date: None,
            dates: vec![],
            geo_loc: Some(Location {
                address_lines: vec!["MAIN STREET".to_string()],
                town: Some("SEDBERGH".to_string()),
                county: Some("CUMBRIA".to_string()),
                postcode: Some("LA10 5AA".to_string()),
                country: Some("ENGLAND".to_string()),
                uprn: None,
            }),
            contacts: vec![],
            roles: vec![OdsRole {
                id: role_code.to_string(),
                code: Some(role_code.to_string()),
                display_name: Some(role_name.to_string()),
                unique_role_id: format!("uid_{code}"),
                primary_role: true,
                status: "active".to_string(),
                dates: vec![],
            }],
            relationships: vec![],
            successors: vec![],
        });
    }

    // 4. Surrey record (SUR01) for location precedence check
    records.push(OdsRecord {
        ods_code: "SUR01".to_string(),
        name: "SURREY MEDICAL CLINIC".to_string(),
        status: "active".to_string(),
        role: "GP Practice".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["HIGH STREET".to_string()],
            town: Some("GUILDFORD".to_string()),
            county: Some("SURREY".to_string()),
            postcode: Some("GU1 1AA".to_string()),
            country: Some("ENGLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO76".to_string(),
            code: Some("RO76".to_string()),
            display_name: Some("GP Practice".to_string()),
            unique_role_id: "uid_sur01".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    // 5. 3-char town records (ELY, AYR)
    records.push(OdsRecord {
        ods_code: "ELY01".to_string(),
        name: "ST MARY'S PHARMACY (ELY)".to_string(),
        status: "active".to_string(),
        role: "Pharmacy".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "site".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["MARKET SQUARE".to_string()],
            town: Some("ELY".to_string()),
            county: Some("CAMBRIDGESHIRE".to_string()),
            postcode: Some("CB7 4DL".to_string()),
            country: Some("ENGLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO182".to_string(),
            code: Some("RO182".to_string()),
            display_name: Some("Pharmacy".to_string()),
            unique_role_id: "uid_ely01".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    records.push(OdsRecord {
        ods_code: "AYR01".to_string(),
        name: "BOOTS OPTICIANS (AYR)".to_string(),
        status: "active".to_string(),
        role: "Pharmacy".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "site".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["HIGH STREET".to_string()],
            town: Some("AYR".to_string()),
            county: Some("AYRSHIRE".to_string()),
            postcode: Some("KA7 1TN".to_string()),
            country: Some("SCOTLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO182".to_string(),
            code: Some("RO182".to_string()),
            display_name: Some("Pharmacy".to_string()),
            unique_role_id: "uid_ayr01".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    // 6. RJZ - KING'S COLLEGE HOSPITAL NHS FOUNDATION TRUST (NHS Trust with RO197, RO7, RO177)
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
        geo_loc: Some(Location {
            address_lines: vec!["DENMARK HILL".to_string()],
            town: Some("LONDON".to_string()),
            county: Some("GREATER LONDON".to_string()),
            postcode: Some("SE5 9RS".to_string()),
            country: Some("ENGLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![
            OdsRole {
                id: "RO197".to_string(),
                code: Some("RO197".to_string()),
                display_name: Some("NHS Trust".to_string()),
                unique_role_id: "uid_rjz1".to_string(),
                primary_role: true,
                status: "active".to_string(),
                dates: vec![],
            },
            OdsRole {
                id: "RO7".to_string(),
                code: Some("RO7".to_string()),
                display_name: Some("Hospice".to_string()),
                unique_role_id: "uid_rjz2".to_string(),
                primary_role: false,
                status: "active".to_string(),
                dates: vec![],
            },
            OdsRole {
                id: "RO177".to_string(),
                code: Some("RO177".to_string()),
                display_name: Some("Prescribing Cost Centre".to_string()),
                unique_role_id: "uid_rjz3".to_string(),
                primary_role: false,
                status: "active".to_string(),
                dates: vec![],
            },
        ],
        relationships: vec![],
        successors: vec![],
    });

    // 7. Royal Free records (RAL and RAL01) for ranking and sorting
    records.push(OdsRecord {
        ods_code: "RAL".to_string(),
        name: "ROYAL FREE LONDON NHS FOUNDATION TRUST".to_string(),
        status: "active".to_string(),
        role: "NHS Trust".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["POND STREET".to_string()],
            town: Some("LONDON".to_string()),
            county: Some("GREATER LONDON".to_string()),
            postcode: Some("NW3 2QG".to_string()),
            country: Some("ENGLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO197".to_string(),
            code: Some("RO197".to_string()),
            display_name: Some("NHS Trust".to_string()),
            unique_role_id: "uid_ral".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    records.push(OdsRecord {
        ods_code: "RAL01".to_string(),
        name: "ROYAL FREE HOSPITAL".to_string(),
        status: "active".to_string(),
        role: "NHS Trust Site".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "site".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["POND STREET".to_string()],
            town: Some("LONDON".to_string()),
            county: Some("GREATER LONDON".to_string()),
            postcode: Some("NW3 2QG".to_string()),
            country: Some("ENGLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO198".to_string(),
            code: Some("RO198".to_string()),
            display_name: Some("NHS Trust Site".to_string()),
            unique_role_id: "uid_ral01".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    // 8. A85619 - Multi-role with low-signal RO177 + RO72 for "+1" preservation test
    records.push(OdsRecord {
        ods_code: "A85619".to_string(),
        name: "DUMMY PRACTICE A85619".to_string(),
        status: "active".to_string(),
        role: "Other Prescribing Cost Centre".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["NORTH ROAD".to_string()],
            town: Some("NEWCASTLE".to_string()),
            county: Some("TYNE AND WEAR".to_string()),
            postcode: Some("NE1 1AA".to_string()),
            country: Some("ENGLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![
            OdsRole {
                id: "RO72".to_string(),
                code: Some("RO72".to_string()),
                display_name: Some("Other Prescribing Cost Centre".to_string()),
                unique_role_id: "uid_a85619_1".to_string(),
                primary_role: true,
                status: "active".to_string(),
                dates: vec![],
            },
            OdsRole {
                id: "RO177".to_string(),
                code: Some("RO177".to_string()),
                display_name: Some("Prescribing Cost Centre".to_string()),
                unique_role_id: "uid_a85619_2".to_string(),
                primary_role: false,
                status: "active".to_string(),
                dates: vec![],
            },
        ],
        relationships: vec![],
        successors: vec![],
    });

    // 9. Dental and national GP practice records
    records.push(OdsRecord {
        ods_code: "DEN02".to_string(),
        name: "PRIVATE DENTAL CLINIC".to_string(),
        status: "active".to_string(),
        role: "Private Dental Practice".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "site".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["CITY ROAD".to_string()],
            town: Some("MANCHESTER".to_string()),
            county: Some("GREATER MANCHESTER".to_string()),
            postcode: Some("M1 1AA".to_string()),
            country: Some("ENGLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO65".to_string(),
            code: Some("RO65".to_string()),
            display_name: Some("Private Dental Practice".to_string()),
            unique_role_id: "uid_den02".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    records.push(OdsRecord {
        ods_code: "SCOT01".to_string(),
        name: "SCOTTISH PRACTICE 01".to_string(),
        status: "active".to_string(),
        role: "Scottish GP Practice".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["PRINCES STREET".to_string()],
            town: Some("EDINBURGH".to_string()),
            county: Some("MIDLOTHIAN".to_string()),
            postcode: Some("EH1 1AA".to_string()),
            country: Some("SCOTLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO227".to_string(),
            code: Some("RO227".to_string()),
            display_name: Some("Scottish GP Practice".to_string()),
            unique_role_id: "uid_scot01".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    records.push(OdsRecord {
        ods_code: "NI01".to_string(),
        name: "NORTHERN IRELAND PRACTICE 01".to_string(),
        status: "active".to_string(),
        role: "Northern Ireland GP Practice".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["DONEGALL SQUARE".to_string()],
            town: Some("BELFAST".to_string()),
            county: Some("ANTRIM".to_string()),
            postcode: Some("BT1 1AA".to_string()),
            country: Some("NORTHERN IRELAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO315".to_string(),
            code: Some("RO315".to_string()),
            display_name: Some("Northern Ireland GP Practice".to_string()),
            unique_role_id: "uid_ni01".to_string(),
            primary_role: true,
            status: "active".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    // 10. Inactive FAH record for code hint test
    records.push(OdsRecord {
        ods_code: "FAH".to_string(),
        name: "LEGACY CLINIC FAH".to_string(),
        status: "inactive".to_string(),
        role: "GP Practice".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["OLD STREET".to_string()],
            town: Some("LONDON".to_string()),
            county: Some("GREATER LONDON".to_string()),
            postcode: Some("EC1A 1AA".to_string()),
            country: Some("ENGLAND".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO76".to_string(),
            code: Some("RO76".to_string()),
            display_name: Some("GP Practice".to_string()),
            unique_role_id: "uid_fah".to_string(),
            primary_role: true,
            status: "inactive".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![],
    });

    // 11. Inactive CLWYD with 5 successors starting with CONWY UA (for "CLWYD → CONWY UA +4")
    records.push(OdsRecord {
        ods_code: "CLWYD".to_string(),
        name: "CLWYD".to_string(),
        status: "inactive".to_string(),
        role: "GP Practice".to_string(),
        parent_organisation: None,
        region_code: None,
        root: None,
        assigning_authority_name: None,
        record_class: "org".to_string(),
        last_change_date: None,
        dates: vec![],
        geo_loc: Some(Location {
            address_lines: vec!["COUNTY HALL".to_string()],
            town: Some("MOLD".to_string()),
            county: Some("FLINTSHIRE".to_string()),
            postcode: Some("CH7 1AA".to_string()),
            country: Some("WALES".to_string()),
            uprn: None,
        }),
        contacts: vec![],
        roles: vec![OdsRole {
            id: "RO76".to_string(),
            code: Some("RO76".to_string()),
            display_name: Some("GP Practice".to_string()),
            unique_role_id: "uid_clwyd".to_string(),
            primary_role: true,
            status: "inactive".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![
            OdsSuccessor {
                unique_succ_id: "clwyd_s1".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("1996-04-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "CONWY01".to_string(),
                    name: Some("CONWY UA".to_string()),
                    ..Default::default()
                },
            },
            OdsSuccessor {
                unique_succ_id: "clwyd_s2".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("1996-04-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "SUCC02".to_string(),
                    name: Some("DENBIGHSHIRE UA".to_string()),
                    ..Default::default()
                },
            },
            OdsSuccessor {
                unique_succ_id: "clwyd_s3".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("1996-04-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "SUCC03".to_string(),
                    name: Some("FLINTSHIRE UA".to_string()),
                    ..Default::default()
                },
            },
            OdsSuccessor {
                unique_succ_id: "clwyd_s4".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("1996-04-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "SUCC04".to_string(),
                    name: Some("WREXHAM UA".to_string()),
                    ..Default::default()
                },
            },
            OdsSuccessor {
                unique_succ_id: "clwyd_s5".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("1996-04-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "SUCC05".to_string(),
                    name: Some("POWYS UA".to_string()),
                    ..Default::default()
                },
            },
        ],
    });

    let clwyd_succs = vec![
        ("CONWY01", "CONWY UA"),
        ("SUCC02", "DENBIGHSHIRE UA"),
        ("SUCC03", "FLINTSHIRE UA"),
        ("SUCC04", "WREXHAM UA"),
        ("SUCC05", "POWYS UA"),
    ];
    for (code, name) in clwyd_succs {
        records.push(OdsRecord {
            ods_code: code.to_string(),
            name: name.to_string(),
            status: "active".to_string(),
            role: "GP Practice".to_string(),
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
                id: "RO76".to_string(),
                code: Some("RO76".to_string()),
                display_name: Some("GP Practice".to_string()),
                unique_role_id: format!("uid_{code}"),
                primary_role: true,
                status: "active".to_string(),
                dates: vec![],
            }],
            relationships: vec![],
            successors: vec![],
        });
    }

    // 12. Inactive 001 with 5 successors (016, 018, 020, 022, 024)
    records.push(OdsRecord {
        ods_code: "001".to_string(),
        name: "LEGACY ORG 001".to_string(),
        status: "inactive".to_string(),
        role: "GP Practice".to_string(),
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
            id: "RO76".to_string(),
            code: Some("RO76".to_string()),
            display_name: Some("GP Practice".to_string()),
            unique_role_id: "uid_001".to_string(),
            primary_role: true,
            status: "inactive".to_string(),
            dates: vec![],
        }],
        relationships: vec![],
        successors: vec![
            OdsSuccessor {
                unique_succ_id: "succ_001_1".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("2010-01-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "016".to_string(),
                    name: Some("SUCCESSOR 016".to_string()),
                    ..Default::default()
                },
            },
            OdsSuccessor {
                unique_succ_id: "succ_001_2".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("2010-01-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "018".to_string(),
                    name: Some("SUCCESSOR 018".to_string()),
                    ..Default::default()
                },
            },
            OdsSuccessor {
                unique_succ_id: "succ_001_3".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("2010-01-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "020".to_string(),
                    name: Some("SUCCESSOR 020".to_string()),
                    ..Default::default()
                },
            },
            OdsSuccessor {
                unique_succ_id: "succ_001_4".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("2010-01-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "022".to_string(),
                    name: Some("SUCCESSOR 022".to_string()),
                    ..Default::default()
                },
            },
            OdsSuccessor {
                unique_succ_id: "succ_001_5".to_string(),
                succ_type: "Successor".to_string(),
                dates: vec![OdsDate {
                    date_type: "Legal".to_string(),
                    start: Some("2010-01-01".to_string()),
                    end: None,
                }],
                target: OdsRelationshipTarget {
                    ods_code: "024".to_string(),
                    name: Some("SUCCESSOR 024".to_string()),
                    ..Default::default()
                },
            },
        ],
    });

    for code in ["016", "018", "020", "022", "024"] {
        records.push(OdsRecord {
            ods_code: code.to_string(),
            name: format!("SUCCESSOR {code}"),
            status: "active".to_string(),
            role: "GP Practice".to_string(),
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
                id: "RO76".to_string(),
                code: Some("RO76".to_string()),
                display_name: Some("GP Practice".to_string()),
                unique_role_id: format!("uid_{code}"),
                primary_role: true,
                status: "active".to_string(),
                dates: vec![],
            }],
            relationships: vec![],
            successors: vec![],
        });
    }

    let mut prov = OdsProvenance::default();
    prov.trud_release_date = Some("2026-07-31".to_string());

    let edges = build_succession_edges(&records);
    let (succ_closures, pred_closures) = compute_transitive_closures(&records, &edges);
    export_orgs(&dir, &records, &succ_closures, &pred_closures, Some(&prov)).expect("export orgs");
    export_orgs_all(
        &dir,
        &records,
        &succ_closures,
        &pred_closures,
        Some(&prov),
    )
    .expect("export orgs_all");
    export_roles(&dir, &records, Some(&prov)).expect("export roles");
    export_relationships(&dir, &records, Some(&prov)).expect("export relationships");
    export_successions(&dir, &records, Some(&prov)).expect("export successions");

    (tmp, dir)
}
