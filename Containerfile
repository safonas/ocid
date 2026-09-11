# syntax=docker/dockerfile:1.7
#
# Multi-stage build for ocid.
#
#   podman build -t ocid .
#   podman run -d --name ocid -p 5050:5050 -v ocid-data:/data ocid
#   podman exec ocid ocictl status
#
# Base images are pinned by digest for reproducible builds; the tag is kept
# for readability. Override with --build-arg if needed.
ARG BUILDER_IMAGE=docker.io/library/rust:1.98.1-slim-bookworm@sha256:ebd900bae66fd508b466cef82d64a83a5fb34682e4c8b2797a42908bddc95a57
ARG RUNTIME_IMAGE=docker.io/library/debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171
ARG SOURCE_DATE_EPOCH=1726012800

# ---------------------------------------------------------------------------
# builder
# ---------------------------------------------------------------------------
FROM ${BUILDER_IMAGE} AS builder

ENV SOURCE_DATE_EPOCH=1726012800
ENV RUSTFLAGS="--remap-path-prefix=/src=/build"

RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# Cache dependency compilation separately from application sources.
COPY Cargo.toml Cargo.lock* ./
COPY crates/ocid-core/Cargo.toml crates/ocid-core/Cargo.toml
COPY crates/ocid/Cargo.toml      crates/ocid/Cargo.toml
COPY crates/ocictl/Cargo.toml    crates/ocictl/Cargo.toml
RUN mkdir -p crates/ocid-core/src crates/ocid/src crates/ocictl/src \
 && echo '' > crates/ocid-core/src/lib.rs \
 && echo 'fn main() {}' > crates/ocid/src/main.rs \
 && echo 'fn main() {}' > crates/ocictl/src/main.rs \
 && (cargo build --release --locked 2>/dev/null || cargo build --release) \
 && rm -rf crates/*/src

COPY crates ./crates
# Touch sources with deterministic timestamp so cargo notices real files replaced stubs.
RUN find crates -name '*.rs' -exec touch -d @${SOURCE_DATE_EPOCH} {} + && cargo build --release \
 && strip target/release/ocid target/release/ocictl

# ---------------------------------------------------------------------------
# runtime
# ---------------------------------------------------------------------------
FROM ${RUNTIME_IMAGE} AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 1000 --home /data --create-home ocid

COPY --from=builder /src/target/release/ocid   /usr/local/bin/ocid
COPY --from=builder /src/target/release/ocictl /usr/local/bin/ocictl

USER ocid
WORKDIR /data
ENV OCID_HOME=/data
# Inside a container the registry must bind all interfaces to be reachable via -p.
ENV OCID_LISTEN=0.0.0.0:5050
VOLUME ["/data"]

# 5050: local OCI registry + control API + /metrics
EXPOSE 5050/tcp

# The daemon initialises an identity on first start if /data is empty.
ENTRYPOINT ["/usr/local/bin/ocid"]
