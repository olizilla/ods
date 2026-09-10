# tests/repro/Dockerfile
#
# Reproducible build verification for `ods make` across platforms (linux/amd64).
#
# Build image:
#   docker build --platform linux/amd64 -t ods-repro:1.94.1 -f tests/repro/Dockerfile .
#
# Run container against mounted release directory and output directory:
#   docker run --rm --platform linux/amd64 \
#     -v "$(pwd)/ods_data/releases/2026-08-28:/release:ro" \
#     -v "/tmp/repro-docker:/out" \
#     ods-repro:1.94.1
#
ARG RUST_VERSION=1.94.1-bookworm
FROM --platform=linux/amd64 rust:${RUST_VERSION} AS builder

WORKDIR /src

# Ensure cmake and git are available for build scripts (libz-ng-sys requires cmake)
RUN apt-get update && apt-get install -y cmake git && rm -rf /var/lib/apt/lists/*

# Allow git in build.rs to read repo metadata without dubious ownership warnings
RUN git config --global --add safe.directory '*'

# Copy repository sources including .git for build.rs to read git SHA and dirty status
COPY . .

# Reset any host-transferred working changes so git status --porcelain is clean at HEAD commit
RUN git reset --hard HEAD

# Build release binary
RUN cargo build --release

ENTRYPOINT ["/bin/bash", "-c", "/src/target/release/ods make --input /release/trud/hscorgrefdataxml_data_8.0.0_20260828000001.zip --output /out && /src/target/release/ods make oci --input /out"]
