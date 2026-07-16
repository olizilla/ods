# ods - NHS ODS Semantic Compiler

A high-performance CLI tool written in Rust for parsing highly normalized, fragmented NHS Organisation Data Service (ODS) TRUD XML schemas (`HSCOrgRefData` v2.0.0) and compiling them into denormalized, flat, downstream open formats.

## Getting Started

### Prerequisites

*   Rust toolchain (Cargo, rustc 2021 edition)

### Building

Build the release binary:

```bash
cargo build --release
```

The compiled binary will be available at `./target/release/ods`.

### Testing

Run all tests (unit + pipeline integration):

```bash
cargo test
```

The integration tests in `tests/pipeline.rs` run the full compile → parquet → okf
pipeline against the fixture in `tests/fixtures/` and assert on typed Rust structs,
replacing the old `verify_compilation.sh` script.

## Commands

### compile

Processes ODS TRUD XML data sequentially in a single memory-efficient streaming pass using `quick-xml`. Emits a canonical Newline Delimited JSON (NDJSON) file.

```bash
ods compile --input <input_dir_or_xml_file> --output <output_dir>
```

### parquet

Converts the compiled NDJSON intermediate file into three relational, sorted Parquet files (`orgs.parquet`, `roles.parquet`, `rels.parquet`) in the output directory. It consolidates addresses, splits dates into legal and operational start/end timestamps, and flattens arrays/structs into primitive columns.

```bash
ods parquet --input <ndjson_file> --output <parquet_output_dir>
```

### okf

Exports compiled Parquet files directly into a single, high-performance compressed Open Knowledge Format (OKF) `wiki.zip` archive containing Markdown files. It parallelizes the page rendering using Rayon, performs fast in-memory joins on roles/relationships/successors, and streams them buffered to a single file.

```bash
ods okf --input <parquet_dir> --output <zip_output_file>
```

## Querying with DuckDB

You can query the exported Parquet files directly from your terminal using `duckdb`.

### Find all active GP Practices
```bash
duckdb -c "
SELECT name, ods_code, postcode, telephone, pcn, icb
FROM 'dist/orgs.parquet'
WHERE status = 'Active'
  AND role IN ('Prescribing Cost Centre', 'General Practice')
ORDER BY ods_code;
"
```

### Query an organization's relationship links
```bash
duckdb -c "
SELECT rel_type, target, target_code, status
FROM 'dist/rels.parquet'
WHERE source_code = 'Y01234';
"
```
