# ODS CLI Development Roadmap

This document outlines the milestones and roadmap for transitioning the `ods` CLI from a two-pass NDJSON-based system to a hybrid auto-bootstrapping Parquet-based architecture.

---

## Roadmap Milestones

### Milestone 1: Re-architect `ods okf` to consume Parquet
*   **Goal**: Enable generating the OKF Wiki directly from `orgs.parquet`, `roles.parquet`, and `rels.parquet` instead of the 500MB `ods.ndjson` file.
*   **Benefits**:
    *   Saves disk space and speeds up OKF generation.
    *   Proves that Parquet contains all the necessary data to reconstruct the entire ODS universe.

### Milestone 2: Define a Local Cache Directory
*   **Goal**: Establish a standard cache directory (e.g. `~/.cache/ods/` or platform-specific equivalent) where the CLI can search for compiled Parquet files.
*   **Benefits**:
    *   Removes the need for users to specify `--input` and `--output` paths on every command.

### Milestone 3: Implement Auto-Bootstrap Fetcher
*   **Goal**: When a user runs a query or generates OKF Wiki without a local cache, the CLI automatically downloads the compressed Parquet files (~30MB) from the latest public GitHub Releases in under 2 seconds.
*   **Benefits**:
    *   Eliminates the requirement for idle curious users to register on TRUD and download the 1.5GB XML.

### Milestone 4: Add `ods query` and TUI Search
*   **Goal**: Implement local querying against the cached Parquet files using a fast column scanner or an embedded engine, followed by a text-based user interface (TUI) for interactive browsing.
