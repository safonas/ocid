# Development happens inside podman; nothing but podman + just is required on the host.
#
#   just              list recipes
#   just sync         switch to main and fast-forward it to github/main
#   just build        incremental debug build (cached registry + target volumes)
#   just release      optimized build
#   just check        cargo check
#   just test         cargo test
#   just fmt          cargo fmt
#   just fmt-check    cargo fmt --check (pre-commit)
#   just clippy       cargo clippy, warnings are errors
#   just run ARGS     run the ocid daemon on the host network (state: ./.dev/ocid-home; OCID_HOME= for more nodes)
#   just ctl ARGS     run ocictl inside the builder against the same state
#   just shell        interactive shell in the build container
#   just bin          copy the debug binaries to ./bin/{ocid,ocictl}
#   just e2e          build, then run scripts/e2e.sh (two nodes on this host; needs podman, curl, jq)
#   just ext-install  npm install for the Podman Desktop extension (wolfi node container)
#   just ext-build    build the extension (backend dist/ + webview media/)
#   just ext-check    typecheck the extension (tsc + svelte-check)
#   just ext-test     unit tests for extension internals (no daemon)
#   just ext-smoke    run the extension client against a throwaway daemon
#   just ext-image    build the extension OCI artifact (for the catalog)
#   just ci           fmt-check + clippy + test + e2e
#   just hooks        install git hooks via pre-commit (fmt/clippy/just-fmt on commit, tests on push)
#   just image        build the runtime container image (Containerfile)
#   just pkg          build .deb + .rpm for the host arch into ./dist (via nfpm)
#   just brew         verify the Homebrew tap formula via brew audit
#   just brew-local   brew-install the current working tree (no tag needed) for local testing
#   just cut-release VER     validate CI, bump version, push release branch, open PR (manual merge)
#   just publish-release VER tag the merge, tap update, draft release + CI publish
#   just republish VER       retry a failed release CI run (re-dispatch the draft build)
#   just clean        remove target volume + ./bin + ./dist
#
# All dev recipes bind-mount the source tree with the shared SELinux label
# (`:z`, not a private `:Z` — per-container categories would break a running
# `just run` daemon whenever another recipe relabels the tree under it) and
# keep the cargo registry and the target dir in named volumes so rebuilds
# are fast.

set shell := ["bash", "-euo", "pipefail", "-c"]
# `mod` needs 1.31+, `set lazy` 1.47+: fail fast with a clear error on old just.
set minimum-version := "1.47.0"
# Only evaluate variables a recipe actually uses (keeps `nfpm_arch`'s
# `error()` below from aborting unrelated invocations like `just --list`).
set lazy

podman_bin := require(env("PODMAN", "podman"))
podman := podman_bin
# Builder images pinned by digest for reproducible builds (override via
# RUST_IMAGE / NODE_IMAGE env). Wolfi-based (Chainguard); the rust image
# ships the same rustc 1.98.1 as the previous Debian pin. NOTE: switching
# the rust image invalidates the target cache (std metadata differs per
# vendor build) — remove the ocid-target volume after a toolchain change.
rust_image := env("RUST_IMAGE", "cgr.dev/chainguard/rust@sha256:635c2f1ae6306ebcbeda3857013f065be7e1ed5dff0f8aff7a6a7a1bce459cac")
image := env("IMAGE", "localhost/ocid:dev")
project := "ocid"
vol_registry := project + "-cargo-registry"
vol_target := project + "-target"
# Chainguard images run as a non-root uid by default; map the invoking user
# in (portable across rootless and rootful podman with --userns=keep-id).
host_uid := `id -u`
host_gid := `id -g`

# Common `podman run` invocation for the build container. `--entrypoint ""`
# clears the image's rustc entrypoint so the recipe's command runs directly;
# CARGO_HOME lives under /tmp (always writable) with the registry cache in
# the named volume, and HOME=/tmp keeps rustup from dropping a .rustup into
# the bind-mounted source tree.
_builder := podman + " run --rm" + " --userns=keep-id" + " --user " + host_uid + ":" + host_gid + ' --entrypoint ""' + " -e HOME=/tmp" + " -e CARGO_HOME=/tmp/cargo" + " -e CARGO_TARGET_DIR=/target" + " -e CARGO_TERM_COLOR=always" + " -e RUST_BACKTRACE=1" + " -v " + justfile_directory() + ":/src:z" + " -v " + vol_registry + ":/tmp/cargo/registry" + " -v " + vol_target + ":/target" + " -w /src"

builder := _builder + " " + rust_image
builder_tty := _builder + " -it " + rust_image
# Builder variant on the host network, for running the daemon/CLI: a
# `just run` daemon then binds the host's 127.0.0.1:5050 (reachable from
# the Podman Desktop extension and podman) and mDNS discovery works.
builder_net_tty := _builder + " --network=host -it " + rust_image

_default:
    @just --list --unsorted

# Ensure named cache volumes exist.
[private]
volumes:
    @{{ podman }} volume exists {{ vol_registry }} || {{ podman }} volume create {{ vol_registry }} >/dev/null
    @{{ podman }} volume exists {{ vol_target }}   || {{ podman }} volume create {{ vol_target }}   >/dev/null

# Switch to main and fast-forward it to github/main. Refuses on a dirty tree.
# Nothing in the release flow commits to main, so this is always a plain
# fast-forward; anything else means manual work that needs a human decision.
[group('dev')]
sync:
    #!/usr/bin/env bash
    set -euo pipefail
    git diff --quiet && git diff --cached --quiet || { echo "error: working tree has uncommitted changes" >&2; exit 1; }
    git remote get-url github >/dev/null 2>&1 || { echo "error: no 'github' remote (fix: git remote add github \"\$(git remote get-url origin)\")" >&2; exit 1; }
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
# Host networking: the registry/control API binds the host's 127.0.0.1:5050,
# where the Podman Desktop extension, podman and `ocictl` expect it; mDNS
# LAN discovery also works. Daemon flags pass straight through, e.g.
# `just run --no-relay` or `just run --listen 127.0.0.1:15061`.
# Set OCID_HOME (relative to the repo root) for additional local nodes —
# the two-node demo from the README:
#   OCID_HOME=.dev/node-b just run --listen 127.0.0.1:15061 --peer <ticket>
# Notes: the daemon persists a --listen override into that home's config.toml
# (rm -rf the home to reset), and if the host's 5050 is already taken (e.g. by
# an installed ocid service) stop it or pass --listen — the bind will fail.
[group('dev')]
run *ARGS: volumes
    #!/usr/bin/env bash
    set -euo pipefail
    home="${OCID_HOME:-.dev/ocid-home}"
    case "$home" in /*) echo "error: OCID_HOME must be relative to the repo root" >&2; exit 1;; esac
    mkdir -p "$home"
    {{ builder_net_tty }} sh -c 'cargo build -q && OCID_HOME="/src/'"$home"'" /target/debug/ocid {{ ARGS }}'

# Run ocictl inside the builder container against a `just run` daemon
# (same home; override with OCID_HOME= as for `just run`).
[group('dev')]
ctl *ARGS: volumes
    #!/usr/bin/env bash
    set -euo pipefail
    home="${OCID_HOME:-.dev/ocid-home}"
    case "$home" in /*) echo "error: OCID_HOME must be relative to the repo root" >&2; exit 1;; esac
    mkdir -p "$home"
    {{ builder_net_tty }} sh -c 'cargo build -q -p ocictl && OCID_HOME="/src/'"$home"'" /target/debug/ocictl {{ ARGS }}'

# Interactive shell in the build container.
[group('dev')]
shell: volumes
    {{ builder_tty }} bash

# Copy the freshly built debug binaries out of the target volume into ./bin.
[group('dev')]
bin: build
    @mkdir -p bin
    {{ podman }} run --rm --userns=keep-id --user {{ host_uid }}:{{ host_gid }} --entrypoint "" \
        -v {{ vol_target }}:/target \
        -v {{ justfile_directory() }}/bin:/out:z \
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

# Native packages, container image and brews live in the `packaging` submodule.
[group('package')]
mod packaging 'just/packaging.just'

# Podman Desktop extension recipes live in the `ext` submodule.
mod ext 'just/extension.just'

# Shim: `just packaging pkg` (release binaries are built first via `release`).
[group('package')]
pkg: release packaging::pkg
# Shim: `just packaging brew`.
[group('package')]
brew: packaging::brew
# Shim: `just packaging brew-local`.
[group('package')]
brew-local: packaging::brew-local

# Shims for the extension module (`just ext build` also works).
[group('extension')]
ext-install: ext::install
# Shim: `just ext build`.
[group('extension')]
ext-build: ext::build
# Shim: `just ext check`.
[group('extension')]
ext-check: ext::check
# Shim: `just ext test` (unit tests, no daemon needed).
[group('extension')]
ext-test: ext::test
# Shim: `just ext smoke` (debug binaries are built first via `just bin`).
[group('extension')]
ext-smoke: bin ext::smoke
# Shim: `just ext image` (single-arch local build; the extension workflow
# publishes the multi-arch artifact via a pinned buildah container).
[group('extension')]
ext-image: ext::image
# Shim: `just ext clean`.
[group('extension')]
ext-clean: ext::clean

# PR-based releases live in the `ship` submodule.
[group('release')]
mod ship 'just/ship.just'

# Shim: `just ship cut-release`.
[group('release')]
cut-release VERSION: (ship::cut-release VERSION)
# Shim: `just ship publish-release`.
[group('release')]
publish-release VERSION: (ship::publish-release VERSION)
# Shim: `just ship republish` (retry a failed release CI run).
[group('release')]
republish VERSION: (ship::republish VERSION)
# Alias for publish-release (confirmation happens there).
[group('release')]
publish VERSION: (ship::publish-release VERSION)

# Remove target volume and ./bin.
[confirm("Delete the target volume, ./bin and ./dist?")]
[group('dev')]
clean:
    -{{ podman }} volume rm {{ vol_target }}
    rm -rf bin dist
