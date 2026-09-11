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
#   just pkg          build .deb + .rpm for the host arch into ./dist (via nfpm)
#   just brew         verify the Homebrew tap formula (checkout in .dev, install from local file)
#   just brew-local   brew-install the current working tree (no tag needed) for local testing
#   just clean        remove target volume + ./bin + ./dist
#
# All dev recipes bind-mount the source tree (:Z for SELinux) and keep the
# cargo registry and the target dir in named volumes so rebuilds are fast.

set shell := ["bash", "-euo", "pipefail", "-c"]

podman       := env("PODMAN", "podman")
# Builder image pinned by digest for reproducible builds (override via RUST_IMAGE env).
rust_image   := env("RUST_IMAGE", "docker.io/library/rust:1.98.1-slim-bookworm@sha256:ebd900bae66fd508b466cef82d64a83a5fb34682e4c8b2797a42908bddc95a57")
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
        {{rust_image}} sh -c 'for b in ocid ocictl ocitop; do cp /target/debug/$b /out/.$b.new && mv -f /out/.$b.new /out/$b; done'
    @echo "-> bin/ocid bin/ocictl bin/ocitop"

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

nfpm_image := "ghcr.io/goreleaser/nfpm:v2.47.0@sha256:a662cb167d7b6d3a83920c83d76b12d02b8ac5dd2c13e5c62c15270b23f6df0c"

# Build .deb + .rpm for the host architecture into ./dist (needs release binaries).
pkg: release
    #!/usr/bin/env bash
    set -euo pipefail
    ver=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
    case "$(uname -m)" in
        x86_64) nfpm_arch=amd64 ;;
        aarch64) nfpm_arch=arm64 ;;
        *) echo "unsupported arch: $(uname -m)" >&2; exit 1 ;;
    esac
    stage="dist/stage"
    cfg="dist/nfpm-$nfpm_arch.yaml"
    mkdir -p "$stage"
    {{podman}} run --rm --userns=keep-id \
        -v {{vol_target}}:/target \
        -v {{justfile_directory()}}/dist:/out:Z \
        {{rust_image}} sh -c "cp /target/release/ocid /target/release/ocictl /target/release/ocitop /out/stage/"
    sed -e "s|@@VERSION@@|$ver|g" -e "s|@@ARCH@@|$nfpm_arch|g" -e "s|@@STAGE@@|$stage|g" \
        packaging/nfpm.yaml > "$cfg"
    {{podman}} run --rm \
        -v {{justfile_directory()}}:/work:Z -w /work \
        {{nfpm_image}} package --config "$cfg" --packager deb --target dist/
    {{podman}} run --rm \
        -v {{justfile_directory()}}:/work:Z -w /work \
        {{nfpm_image}} package --config "$cfg" --packager rpm --target dist/
    rm -rf "$stage" "$cfg"
    ls dist/

# Verify the Homebrew tap formula end-to-end. Homebrew only installs formulae
# that live in a tap, so this works inside the tapped checkout of
# safonas/homebrew-tap (`brew --repository safonas/tap`, tapped on demand),
# reset to origin/main every run: bump url/sha256 to the current Cargo.toml
# version (the v<ver> tag must already be on GitHub), reinstall from the tap
# and smoke-test the binaries. Pushing the tap is manual:
#   git -C "$(brew --repository safonas/tap)" commit -am "ocid: bump to vX.Y.Z" && git -C "$(brew --repository safonas/tap)" push
brew:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    command -v brew >/dev/null || { echo "brew not found on PATH" >&2; exit 1; }
    brew tap safonas/tap >/dev/null 2>&1 || true
    tap=$(brew --repository safonas/tap)
    ver=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
    url="https://github.com/safonas/ocid/archive/refs/tags/v$ver.tar.gz"
    git -C "$tap" fetch -q origin && git -C "$tap" checkout -q main && git -C "$tap" reset -q --hard origin/main
    if [ "$(curl -sSL -o /dev/null -w '%{http_code}' "$url")" != "200" ]; then
        echo "tag v$ver not published on GitHub yet (archive 404); push the tag first" >&2
        exit 1
    fi
    sha=$(curl -sSL "$url" | sha256sum | cut -d' ' -f1)
    sed -i -e "s|^  url .*|  url \"$url\"|" -e "s|^  sha256 .*|  sha256 \"$sha\"|" "$tap/Formula/ocid.rb"
    if ! grep -q 'crates/ocitop' "$tap/Formula/ocid.rb"; then
        sed -i -e '/crates\/ocictl/a\    system "cargo", "install", *std_cargo_args(path: "crates/ocitop")' "$tap/Formula/ocid.rb"
    fi
    git -C "$tap" --no-pager diff -- Formula/ocid.rb || true
    brew uninstall --ignore-dependencies ocid >/dev/null 2>&1 || true
    brew install --formula safonas/tap/ocid
    ocid --version; ocictl --version; ocitop --version

# Install the *working tree* through Homebrew for local testing (no tag or
# commit needed). Homebrew 6 refuses loose formula files, so this keeps a
# private local tap `safonas/local` (created on demand, never pushed) whose
# `ocid` formula points at a tarball of the checkout in .dev via file://.
# Replaces any installed `ocid` (from the real tap); `just brew` swaps back.
brew-local:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}"
    command -v brew >/dev/null || { echo "brew not found on PATH" >&2; exit 1; }
    ver=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
    out=".dev/brew"; mkdir -p "$out"
    tarball="$PWD/$out/ocid-$ver-local.tar.gz"
    # tracked + untracked-but-not-ignored files that exist, deterministic layout
    git ls-files -co --exclude-standard -z | while IFS= read -r -d '' f; do [ -e "$f" ] && printf '%s\0' "$f"; done \
        | tar --null -T - --transform "s,^,ocid-$ver/," -czf "$tarball"
    sha=$(sha256sum "$tarball" | cut -d' ' -f1)
    brew tap-new safonas/local --no-git >/dev/null 2>&1 || true
    tap=$(brew --repository safonas/local)
    mkdir -p "$tap/Formula"
    cat >"$tap/Formula/ocid.rb" <<EOF
    class Ocid < Formula
      desc "Local-first, peer-to-peer distribution of OCI container images (LOCAL BUILD)"
      homepage "https://github.com/safonas/ocid"
      url "file://$tarball"
      sha256 "$sha"
      version "$ver-local"
      license "GPL-3.0-or-later"
      depends_on "rust" => :build
      def install
        system "cargo", "install", *std_cargo_args(path: "crates/ocid")
        system "cargo", "install", *std_cargo_args(path: "crates/ocictl")
        system "cargo", "install", *std_cargo_args(path: "crates/ocitop")
      end
      test do
        assert_match "ocid", shell_output("#{bin}/ocid --version")
      end
    end
    EOF
    brew uninstall --ignore-dependencies ocid >/dev/null 2>&1 || true
    brew install --formula safonas/local/ocid
    ocid --version; ocictl --version; ocitop --version

# Remove target volume and ./bin.
clean:
    -{{podman}} volume rm {{vol_target}}
    rm -rf bin dist
