# ODS CLI Development Roadmap

This document outlines the milestones and roadmap for transitioning the `ods` CLI from a two-pass NDJSON-based system to a hybrid auto-bootstrapping Parquet-based architecture.

---

## Roadmap Milestones

### Milestone 1: Re-architect `ods okf` to consume Parquet
*   **Goal**: Enable generating the OKF Wiki directly from `orgs.parquet`, `roles.parquet`, and `rels.parquet` instead of the 500MB `ods.ndjson` file.
*   **Benefits**:
    *   Saves disk space and speeds up OKF generation.
    *   Proves that Parquet contains all the necessary data to reconstruct the entire ODS universe.

### Milestone 2: Integrate Apache DataFusion & Explainable SQL Query Transpiler
*   **Goal**: Replace manual imperative Rust record batch scanners in `ods find` with an embedded Apache DataFusion query engine.
*   **Benefits**:
    *   Adds `--explain` / `--sql` flag to `ods find` to display canonical, DuckDB-compatible SQL queries before execution.
    *   Establishes standard ANSI SQL querying over `orgs.parquet`, `roles.parquet`, `rels.parquet`, and `successors.parquet`.
    *   Powers the new `ods sql` subcommand for ad-hoc user query execution.

### Milestone 3: Zero-Wait Remote Parquet Resolution & Versioned Local Cache
*   **Goal**: Enable OOTB querying against hosted remote Parquet files over HTTP Range Requests (`object_store`), backed by an immutable, versioned local cache (`~/.cache/ods/releases/<release_tag>/`).
*   **Benefits**:
    *   Fresh CLI installs query remote Parquet datasets instantly OOTB (< 300ms) without downloading 30MB upfront files.
    *   Decouples software CLI updates (`brew install ods`) from monthly TRUD dataset releases.
    *   Guarantees 100% reproducible scientific citations by pinning active dataset release versions and notifying users of newer releases via non-intrusive `stderr` notices.

### Milestone 4: Add `ods cache` Management & Interactive TUI
*   **Goal**: Provide explicit local cache management commands (`ods cache status`, `ods cache update`) and an interactive terminal user interface (TUI) for browsing ODS hierarchies.
*   **Benefits**:
    *   Gives users explicit control over dataset upgrades.
    *   Enables fast, offline exploration of NHS ODS entities directly in the terminal.
