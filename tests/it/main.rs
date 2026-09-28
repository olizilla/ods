//! Entry point for the consolidated integration-test binary: one process, one link,
//! every `tests/it/*.rs` file a module below. Run everything with `cargo test`, or one
//! module with `cargo test --test it <module>::`.

mod common;

mod archive_verification_test;
mod audit_archive_test;
mod audit_hash_chain_test;
mod brief_make_and_discovery_acceptance_test;
mod broken_pipe_test;
mod cite_consumer_test;
mod cli_test;
mod datapackage_schema_test;
mod fetch_local_archive_test;
mod find_in_exact_test;
mod find_name_matching_test;
mod find_open_not_active_test;
mod find_output_record_test;
mod find_search_test;
mod find_sql_flag_test;
mod find_succession_test;
mod find_table_footer_test;
mod info_reference_test;
mod info_test;
mod make_oci_source_test;
mod make_oci_test;
mod make_provenance_reporting_test;
mod make_release_test;
mod mirror_to_ods_fyi_test;
mod oci_manifest_golden_test;
mod oci_source_manifest_golden_test;
mod pipeline;
mod provenance_lifecycle_test;
mod pull_index_flag_test;
mod pull_oci_test;
mod pull_test;
mod release_index_test;
mod reproducibility_test;
mod role_test;
mod say_which_release_test;
mod schema_contract_test;
mod snapshots;
mod temporal_parsing_test;
mod trud_attestations_test;
mod trud_projection_test;
mod trud_pull_repair_test;
mod trud_pull_ux_test;
mod use_test;
mod workspace_test;
mod write_where_told_test;
