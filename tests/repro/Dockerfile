# tests/repro/Dockerfile
#
# Reproducible build verification for `ods make` across platforms (linux/amd64).
#
# Build image:
#   docker build --platform linux/amd64 -t ods-repro:1.94.1 -f tests/repro/Dockerfile .
#
# Run container against mounted TRUD archive and output directory:
#   docker run --rm --platform linux/amd64 \
#     -v "$(pwd)/ods_data/releases/2026-08-28/trud:/trud:ro" \
#     -v "/tmp/repro-docker:/out" \
#     ods-repro:1.94.1
#
FROM --platform=linux/amd64 rust:1.94.1-bookworm AS builder

WORKDIR /src

# Allow git in build.rs to read repo metadata without dubious ownership warnings
RUN git config --global --add safe.directory '*'

# Copy repository sources including .git for build.rs to read git SHA and dirty status
COPY . .

# Build release binary
RUN cargo build --release

ENTRYPOINT ["/bin/bash", "-c", "/src/target/release/ods make --input /trud/hscorgrefdataxml_data_8.0.0_20260828000001.zip --output /out && /src/target/release/ods make oci --input /out"]
