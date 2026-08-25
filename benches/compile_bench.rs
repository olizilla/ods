use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ods::commands::ndjson::{convert_parsed_orgs, ParsedOrg, OdsRole, OdsRelationship, OdsRelationshipTarget, ParentOrganisation};
use std::collections::HashMap;

fn create_mock_records(count: usize) -> HashMap<String, ParsedOrg> {
    let mut records = HashMap::new();

    // Add parent/commissioner nodes (e.g. PCN, Trust, ICB)
    records.insert(
        "RO272_parent".to_string(),
        ParsedOrg {
            ods_code: "RO272_parent".to_string(),
            name: "Mock PCN parent".to_string(),
            status: "Active".to_string(),
            role: "PCN".to_string(),
            parent_organisation: None,
            region_code: None,
            root: None,
            assigning_authority_name: None,
            org_record_class: None,
            last_change_date: None,
            dates: vec![],
            geo_loc: None,
            contacts: vec![],
            roles: vec![OdsRole {
                id: "RO272".to_string(),
                code: Some("272".to_string()),
                display_name: Some("Primary Care Network".to_string()),
                unique_role_id: "1".to_string(),
                primary_role: true,
                status: "Active".to_string(),
                dates: vec![],
            }],
            relationships: vec![],
            successors: vec![],
        },
    );

    // Add thousands of GP practice records pointing to this parent
    for i in 0..count {
        let code = format!("Y{:05}", i);
        records.insert(
            code.clone(),
            ParsedOrg {
                ods_code: code.clone(),
                name: format!("Mock GP Practice {}", i),
                status: "Active".to_string(),
                role: "GP Practice".to_string(),
                parent_organisation: Some(ParentOrganisation {
                    ods_code: "RO272_parent".to_string(),
                    name: "Mock PCN parent".to_string(),
                }),
                region_code: None,
                root: None,
                assigning_authority_name: None,
                org_record_class: None,
                last_change_date: None,
                dates: vec![],
                geo_loc: None,
                contacts: vec![],
                roles: vec![OdsRole {
                    id: "RO76".to_string(),
                    code: Some("76".to_string()),
                    display_name: Some("General Practice".to_string()),
                    unique_role_id: "2".to_string(),
                    primary_role: true,
                    status: "Active".to_string(),
                    dates: vec![],
                }],
                relationships: vec![OdsRelationship {
                    id: "RE4".to_string(),
                    display_name: Some("is commissioned by".to_string()),
                    unique_rel_id: "9999".to_string(),
                    status: "Active".to_string(),
                    target: OdsRelationshipTarget {
                        ods_code: "RO272_parent".to_string(),
                        name: Some("Mock PCN parent".to_string()),
                        root: None,
                        assigning_authority_name: None,
                        primary_role_id: None,
                        primary_role_display_name: None,
                        primary_role_unique_role_id: None,
                    },
                    dates: vec![],
                }],
                successors: vec![],
            },
        );
    }

    records
}

fn bench_record_conversion(c: &mut Criterion) {
    // Generate 5,000 mock records
    let records = create_mock_records(5000);

    c.bench_function("record_conversion_5000_records", |b| {
        b.iter_batched(
            || records.clone(),
            |recs| {
                let res = convert_parsed_orgs(black_box(recs));
                black_box(res);
            },
            criterion::BatchSize::SmallInput,
        )
    });
}

criterion_group!(benches, bench_record_conversion);
criterion_main!(benches);

