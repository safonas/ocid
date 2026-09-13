# Development happens inside podman; nothing but podman + just is required on the host.
#
#   just              list recipes
#   just sync         switch to main and fast-forward to github/main
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
#   just hooks        install git hooks via pre-commit (fmt/clippy/just-fmt on commit, tests on push)
#   just image        build the runtime container image (Containerfile)
#   just pkg          build .deb + .rpm for the host arch into ./dist (via nfpm)
#   just android build  cross-compile release binaries for Android arm64 (Termux)
#   just android pkg    package Android arm64 binaries into ./dist/ocid-android-arm64.tar.gz
#   just brew         verify the Homebrew tap formula via brew audit
#   just brew-local   brew-install the current working tree (no tag needed) for local testing
#   just cut-release VER     validate CI, bump version, push release branch, open PR (manual merge)
#   just publish-release VER tag the merge, create GH release, update tap (run after the PR merged)
#   just clean        remove target volume + ./bin + ./dist
#
# All dev recipes bind-mount the source tree (:Z for SELinux) and keep the
# cargo registry and the target dir in named volumes so rebuilds are fast.

set shell := ["bash", "-euo", "pipefail", "-c"]
# `mod` needs 1.31+, `set lazy` 1.47+: fail fast with a clear error on old just.
set minimum-version := "1.47.0"
# Only evaluate variables a recipe actually uses (keeps `nfpm_arch`'s
# `error()` below from aborting unrelated invocations like `just --list`).
set lazy

podman_bin := require(env("PODMAN", "podman"))
podman := podman_bin
# Builder image pinned by digest for reproducible builds (override via RUST_IMAGE env).
rust_image := env("RUST_IMAGE", "docker.io/library/rust:1.98.1-slim-bookworm@sha256:ebd900bae66fd508b466cef82d64a83a5fb34682e4c8b2797a42908bddc95a57")
image := env("IMAGE", "localhost/ocid:dev")
project := "ocid"
vol_registry := project + "-cargo-registry"
vol_target := project + "-target"

# Common `podman run` invocation for the build container.
_builder := podman + " run --rm" + " --userns=keep-id" + " -e CARGO_HOME=/usr/local/cargo" + " -e CARGO_TARGET_DIR=/target" + " -e CARGO_TERM_COLOR=always" + " -e RUST_BACKTRACE=1" + " -v " + justfile_directory() + ":/src:Z" + " -v " + vol_registry + ":/usr/local/cargo/registry" + " -v " + vol_target + ":/target" + " -w /src"

builder := _builder + " " + rust_image
builder_tty := _builder + " -it " + rust_image

_default:
    @just --list --unsorted

# Ensure named cache volumes exist.
[private]
volumes:
    @{{ podman }} volume exists {{ vol_registry }} || {{ podman }} volume create {{ vol_registry }} >/dev/null
    @{{ podman }} volume exists {{ vol_target }}   || {{ podman }} volume create {{ vol_target }}   >/dev/null

# Switch to main and fast-forward it to github/main. Refuses on a dirty tree.
[group('dev')]
sync:
    #!/usr/bin/env bash
    set -euo pipefail
    git diff --quiet && git diff --cached --quiet || { echo "error: working tree has uncommitted changes" >&2; exit 1; }
    git checkout -q main
    git pull github main --ff-only
    git log --oneline -3

# Incremental debug build.
[group('dev')]
build: volumes
    {{ builder }} cargo build

# Optimized build.
[group('dev')]
release: volumes
    {{ builder }} cargo build --release

# Type-check only.
[group('dev')]
check: volumes
    {{ builder }} cargo check

# Run tests.
[group('dev')]
test *ARGS: volumes
    {{ builder }} cargo test {{ ARGS }}

# Format sources.
[group('dev')]
fmt: volumes
    {{ builder }} sh -c 'rustup component add rustfmt >/dev/null 2>&1; cargo fmt'

# Fail if sources are not formatted.
[group('dev')]
fmt-check: volumes
    {{ builder }} sh -c 'rustup component add rustfmt >/dev/null 2>&1; cargo fmt --check'

# Lint; warnings are errors (same as CI).
[group('dev')]
clippy: volumes
    {{ builder }} sh -c 'rustup component add clippy >/dev/null 2>&1; cargo clippy --all-targets -- -D warnings'

# Run the ocid daemon inside the builder container (state under ./.dev/ocid-home).
[group('dev')]
run *ARGS: volumes
    @mkdir -p .dev/ocid-home
    {{ builder_tty }} sh -c 'cargo build -q && OCID_HOME=/src/.dev/ocid-home /target/debug/ocid {{ ARGS }}'

# Run ocictl inside the builder container against ./.dev/ocid-home.
[group('dev')]
ctl *ARGS: volumes
    @mkdir -p .dev/ocid-home
    {{ builder_tty }} sh -c 'cargo build -q -p ocictl && OCID_HOME=/src/.dev/ocid-home /target/debug/ocictl {{ ARGS }}'

# Interactive shell in the build container.
[group('dev')]
shell: volumes
    {{ builder_tty }} bash

# Copy the freshly built debug binaries out of the target volume into ./bin.
[group('dev')]
bin: build
    @mkdir -p bin
    {{ podman }} run --rm --userns=keep-id \
        -v {{ vol_target }}:/target \
        -v {{ justfile_directory() }}/bin:/out:Z \
        {{ rust_image }} sh -c 'for b in ocid ocictl ocitop; do cp /target/debug/$b /out/.$b.new && mv -f /out/.$b.new /out/$b; done'
    @echo "-> bin/ocid bin/ocictl bin/ocitop"

# End-to-end tests: two daemons on the host driven by podman/curl/ocictl.
[group('dev')]
e2e: bin
    scripts/e2e.sh

# Everything CI would run.
[group('dev')]
ci: volumes
    {{ builder }} sh -c 'rustup component add rustfmt clippy >/dev/null 2>&1; cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test'
    just e2e

# Install the git hooks (pre-commit: fmt/clippy/shellcheck/hygiene, pre-push: tests).
[group('dev')]
hooks:
    pre-commit install --install-hooks --hook-type pre-commit --hook-type pre-push

# Build the runtime container image.
# (Implementation lives in the `packaging` submodule.)
[group('package')]
image: packaging::image

# Android arm64 (Termux) cross-build lives in the `android` submodule.
[group('package')]
mod android 'just/android.just'

# Shim: `just android build`.
[group('package')]
build-android: android::build
# Shim: `just android pkg`.
[group('package')]
pkg-android: android::pkg

# Native packages, container image and brews live in the `packaging` submodule.
[group('package')]
mod packaging 'just/packaging.just'

# Shim: `just packaging pkg` (release binaries are built first via `release`).
[group('package')]
pkg: release packaging::pkg
# Shim: `just packaging brew`.
[group('package')]
brew: packaging::brew
# Shim: `just packaging brew-local`.
[group('package')]
brew-local: packaging::brew-local

# PR-based releases live in the `ship` submodule.
[group('release')]
mod ship 'just/ship.just'

# Shim: `just ship cut-release`.
[group('release')]
cut-release VERSION: (ship::cut-release VERSION)
# Shim: `just ship publish-release`.
[group('release')]
publish-release VERSION: (ship::publish-release VERSION)
# Alias for publish-release (confirmation happens there).
[group('release')]
publish VERSION: (ship::publish-release VERSION)

# Remove target volume and ./bin.
[confirm("Delete the target volume, ./bin and ./dist?")]
[group('dev')]
clean:
    -{{ podman }} volume rm {{ vol_target }}
    rm -rf bin dist
