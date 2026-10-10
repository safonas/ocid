# syntax=docker/dockerfile:1.28@sha256:bb22d9815c728170f72750f4e5b0d672e06176142e1d602c7e66c050100b7e5b
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
ARG BUILDER_IMAGE=cgr.dev/chainguard/rust@sha256:582a81742d5038a25c0b07359762953d94dbbe17533ab6db5164882b82a2e15b
# Wolfi rolling base (re-pin regularly for fresh security fixes).
ARG RUNTIME_IMAGE=cgr.dev/chainguard/wolfi-base:latest@sha256:1c451d46a0d5c4e9f2b38e0e8d3e299564a1aa95c21973efcc1980a9d1d2e73e
# Reproducible layer timestamps: the release flow passes the tag commit time
# via --build-arg; this default is the project initial commit epoch.
ARG SOURCE_DATE_EPOCH=1789090756

# ---------------------------------------------------------------------------
# builder
# ---------------------------------------------------------------------------
FROM ${BUILDER_IMAGE} AS builder

# Chainguard images run as a non-root uid by default; the build needs to
# write to /src (COPY'd as root).
USER root

ENV SOURCE_DATE_EPOCH=1789090756
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
# daemon — ocid only: what the extension bundles (`--target daemon`; the
# release workflow assembles the same image from the release binary).
# ---------------------------------------------------------------------------
FROM ${RUNTIME_IMAGE} AS daemon

RUN apk add --no-cache ca-certificates \
  && addgroup -S -g 1000 ocid \
  && adduser -S -D -u 1000 -G ocid -h /data ocid

COPY --from=builder /src/target/release/ocid /usr/local/bin/ocid

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

# ---------------------------------------------------------------------------
# runtime — the default target: daemon plus ocictl/ocitop for
# in-container debugging (`podman exec ocid ocictl status`).
# ---------------------------------------------------------------------------
FROM daemon AS runtime

COPY --from=builder /src/target/release/ocictl /usr/local/bin/ocictl
COPY --from=builder /src/target/release/ocitop /usr/local/bin/ocitop
