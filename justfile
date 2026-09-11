# Development happens inside podman; nothing but podman + just is required on the host.
#
#   just              list recipes
#   just build        incremental debug build (cached registry + target volumes)
#   just release      optimized build
#   just check        cargo check
#   just test         cargo test
#   just fmt          cargo fmt
#   just fmt-check    cargo fmt --check (pre-commit)
#   just clippy       cargo clippy, warnings are errors
#   just run ARGS     run the ocid daemon inside the builder (state under ./.dev/ocid-home)
#   just ctl ARGS     run ocictl inside the builder against the same state
#   just shell        interactive shell in the build container
#   just bin          copy the debug binaries to ./bin/{ocid,ocictl}
#   just e2e          build, then run scripts/e2e.sh (two nodes on this host; needs podman, curl, jq)
#   just ci           fmt-check + clippy + test + e2e
#   just hooks        install git hooks via pre-commit (fmt/clippy on commit, tests on push)
#   just image        build the runtime container image (Containerfile)
#   just clean        remove target volume + ./bin
#
# All dev recipes bind-mount the source tree (:Z for SELinux) and keep the
# cargo registry and the target dir in named volumes so rebuilds are fast.

set shell := ["bash", "-euo", "pipefail", "-c"]

podman       := env("PODMAN", "podman")
rust_image   := env("RUST_IMAGE", "docker.io/library/rust:1.98.1-slim-bookworm")
image        := env("IMAGE", "localhost/ocid:dev")
project      := "ocid"
vol_registry := project + "-cargo-registry"
vol_target   := project + "-target"

# Common `podman run` invocation for the build container.
_builder := podman + " run --rm" \
    + " --userns=keep-id" \
    + " -e CARGO_HOME=/usr/local/cargo" \
    + " -e CARGO_TARGET_DIR=/target" \
    + " -e CARGO_TERM_COLOR=always" \
    + " -e RUST_BACKTRACE=1" \
    + " -v " + justfile_directory() + ":/src:Z" \
    + " -v " + vol_registry + ":/usr/local/cargo/registry" \
    + " -v " + vol_target + ":/target" \
    + " -w /src"

builder     := _builder + " " + rust_image
builder_tty := _builder + " -it " + rust_image

_default:
    @just --list --unsorted

# Ensure named cache volumes exist.
volumes:
    @{{podman}} volume exists {{vol_registry}} || {{podman}} volume create {{vol_registry}} >/dev/null
    @{{podman}} volume exists {{vol_target}}   || {{podman}} volume create {{vol_target}}   >/dev/null

# Incremental debug build.
build: volumes
    {{builder}} cargo build

# Optimized build.
release: volumes
    {{builder}} cargo build --release

# Type-check only.
check: volumes
    {{builder}} cargo check

# Run tests.
test *ARGS: volumes
    {{builder}} cargo test {{ARGS}}

# Format sources.
fmt: volumes
    {{builder}} sh -c 'rustup component add rustfmt >/dev/null 2>&1; cargo fmt'

# Fail if sources are not formatted.
fmt-check: volumes
    {{builder}} sh -c 'rustup component add rustfmt >/dev/null 2>&1; cargo fmt --check'

# Lint; warnings are errors (same as CI).
clippy: volumes
    {{builder}} sh -c 'rustup component add clippy >/dev/null 2>&1; cargo clippy --all-targets -- -D warnings'

# Run the ocid daemon inside the builder container (state under ./.dev/ocid-home).
run *ARGS: volumes
    @mkdir -p .dev/ocid-home
    {{builder_tty}} sh -c 'cargo build -q && OCID_HOME=/src/.dev/ocid-home /target/debug/ocid {{ARGS}}'

# Run ocictl inside the builder container against ./.dev/ocid-home.
ctl *ARGS: volumes
    @mkdir -p .dev/ocid-home
    {{builder_tty}} sh -c 'cargo build -q -p ocictl && OCID_HOME=/src/.dev/ocid-home /target/debug/ocictl {{ARGS}}'

# Interactive shell in the build container.
shell: volumes
    {{builder_tty}} bash

# Copy the freshly built debug binaries out of the target volume into ./bin.
bin: build
    @mkdir -p bin
    {{podman}} run --rm --userns=keep-id \
        -v {{vol_target}}:/target \
        -v {{justfile_directory()}}/bin:/out:Z \
        {{rust_image}} sh -c 'for b in ocid ocictl; do cp /target/debug/$b /out/.$b.new && mv -f /out/.$b.new /out/$b; done'
    @echo "-> bin/ocid bin/ocictl"

# End-to-end tests: two daemons on the host driven by podman/curl/ocictl.
e2e: bin
    scripts/e2e.sh

# Everything CI would run.
ci: volumes
    {{builder}} sh -c 'rustup component add rustfmt clippy >/dev/null 2>&1; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test'
    just e2e

# Install the git hooks (pre-commit: fmt/clippy/shellcheck/hygiene, pre-push: tests).
hooks:
    pre-commit install --install-hooks --hook-type pre-commit --hook-type pre-push

# Build the runtime container image.
image:
    {{podman}} build -t {{image}} -f Containerfile .

# Remove target volume and ./bin.
clean:
    -{{podman}} volume rm {{vol_target}}
    rm -rf bin
