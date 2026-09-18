# syntax=docker/dockerfile:1.7
#
# Multi-stage build for ocid.
#
#   podman build -t ocid .
#   podman run -d --name ocid -p 5050:5050 -v ocid-data:/data ocid
#   podman exec ocid ocictl status
#
# Base images are pinned by digest for reproducible builds; the tag is kept
# for readability. Override with --build-arg if needed. Both are Wolfi-based
# (Chainguard).
ARG BUILDER_IMAGE=cgr.dev/chainguard/rust@sha256:7d70867ec51393a4e04db3a0c4d89d91c7eab1ee0fe4650107116de6d2bea096
# Wolfi rolling base (re-pin regularly for fresh security fixes).
ARG RUNTIME_IMAGE=cgr.dev/chainguard/wolfi-base:latest@sha256:32d119bfa89c4302e0608f5c120b39c6b8f80b592c7fb46aa27008b2669ce220
ARG SOURCE_DATE_EPOCH=[PHONE]

# ---------------------------------------------------------------------------
# builder
# ---------------------------------------------------------------------------
FROM ${BUILDER_IMAGE} AS builder

# Chainguard images run as a non-root uid by default; the build needs to
# write to /src (COPY'd as root).
USER root

ENV SOURCE_DATE_EPOCH=[PHONE]
ENV RUSTFLAGS="--remap-path-prefix=/src=/build"

WORKDIR /src

# Cache dependency compilation separately from application sources.
COPY Cargo.toml Cargo.lock* ./
COPY crates/ocid-core/Cargo.toml crates/ocid-core/Cargo.toml
COPY crates/ocid/Cargo.toml      crates/ocid/Cargo.toml
COPY crates/ocictl/Cargo.toml    crates/ocictl/Cargo.toml
COPY crates/ocitop/Cargo.toml    crates/ocitop/Cargo.toml
RUN mkdir -p crates/ocid-core/src crates/ocid/src crates/ocictl/src crates/ocitop/src \
 && echo '' > crates/ocid-core/src/lib.rs \
 && echo 'fn main() {}' > crates/ocid/src/main.rs \
 && echo 'fn main() {}' > crates/ocictl/src/main.rs \
 && echo 'fn main() {}' > crates/ocitop/src/main.rs \
 && (cargo build --release --locked 2>/dev/null || cargo build --release) \
 && rm -rf crates/*/src

COPY crates ./crates
# Touch sources so cargo sees them as newer than the stub-build artifacts and
# actually rebuilds (a fixed past timestamp would look older and ship stubs).
RUN find crates -name '*.rs' -exec touch {} + && cargo build --release \
 && strip target/release/ocid target/release/ocictl target/release/ocitop

# ---------------------------------------------------------------------------
# runtime
# ---------------------------------------------------------------------------
FROM ${RUNTIME_IMAGE} AS runtime

RUN apk add --no-cache ca-certificates \
 && addgroup -S -g 1000 ocid \
 && adduser -S -D -u 1000 -G ocid -h /data ocid

COPY --from=builder /src/target/release/ocid   /usr/local/bin/ocid
COPY --from=builder /src/target/release/ocictl /usr/local/bin/ocictl
COPY --from=builder /src/target/release/ocitop /usr/local/bin/ocitop

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
