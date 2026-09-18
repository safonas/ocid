# Native packaging submodule: container image, .deb/.rpm, Homebrew.
#
#   just packaging image       build the runtime container image
#   just packaging pkg         .deb + .rpm for the host arch (needs release binaries)
#   just packaging brew        verify the Homebrew tap formula
#   just packaging brew-local  test-install the working tree through a local tap
#
# Invoke from the repo root; `justfile_directory()` still points there, and
# `[no-cd]` keeps each recipe running there too (otherwise just would cd into
# `just/`). Self-contained: just modules cannot see the parent's
# variables/recipes, so settings below mirror the root justfile.

set shell := ["bash", "-euo", "pipefail", "-c"]
set minimum-version := "1.47.0"
# Only evaluate variables a recipe actually uses (keeps `nfpm_arch`'s
# `error()` from aborting unrelated invocations like `just --list`).
set lazy

podman := require(env("PODMAN", "podman"))
# Wolfi-based builder, pinned by digest (mirrors the root justfile; override
# via RUST_IMAGE env).
rust_image := env("RUST_IMAGE", "cgr.dev/chainguard/rust@sha256:635c2f1ae6306ebcbeda3857013f065be7e1ed5dff0f8aff7a6a7a1bce459cac")
image := env("IMAGE", "localhost/ocid:dev")
project := "ocid"
vol_registry := project + "-cargo-registry"
vol_target := project + "-target"
# Chainguard images run as a non-root uid by default; map the invoking user
# in (portable across rootless and rootful podman with --userns=keep-id).
host_uid := `id -u`
host_gid := `id -g`

nfpm_image := "ghcr.io/goreleaser/nfpm:v2.47.0@sha256:a662cb167d7b6d3a83920c83d76b12d02b8ac5dd2c13e5c62c15270b23f6df0c"

# Host -> nfpm arch mapping, resolved by just (not the shell).
nfpm_arch := if arch() == "x86_64" { "amd64" } else if arch() == "aarch64" { "arm64" } else { error("unsupported arch: " + arch()) }

# Build the runtime container image.
[no-cd]
image:
    {{ podman }} build -t {{ image }} -f Containerfile .

# Build .deb + .rpm for the host architecture into ./dist (needs release binaries).
[no-cd]
pkg:
    #!/usr/bin/env bash
    set -euo pipefail
    ver=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
    stage="dist/stage"
    cfg="dist/nfpm-{{ nfpm_arch }}.yaml"
    mkdir -p "$stage"
    {{ podman }} run --rm --userns=keep-id --user {{ host_uid }}:{{ host_gid }} --entrypoint "" \
        -v {{ vol_target }}:/target \
        -v {{ justfile_directory() }}/dist:/out:Z \
        {{ rust_image }} sh -c "cp /target/release/ocid /target/release/ocictl /target/release/ocitop /out/stage/"
    sed -e "s|@@VERSION@@|$ver|g" -e "s|@@ARCH@@|$nfpm_arch|g" -e "s|@@STAGE@@|$stage|g" \
        packaging/nfpm.yaml > "$cfg"
    {{ podman }} run --rm \
        -v {{ justfile_directory() }}:/work:Z -w /work \
        {{ nfpm_image }} package --config "$cfg" --packager deb --target dist/
    {{ podman }} run --rm \
        -v {{ justfile_directory() }}:/work:Z -w /work \
        {{ nfpm_image }} package --config "$cfg" --packager rpm --target dist/
    rm -rf "$stage" "$cfg"
    ls dist/

# Verify the Homebrew tap formula syntax and structure via brew audit.
# Syncs safonas/tap, updates url/sha256/crates to match Cargo.toml,
# and audits the formula. (Local compile/install is testable via just brew-local).
[no-cd]
brew:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{ justfile_directory() }}"
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
    brew audit --formula safonas/tap/ocid
    echo "-> safonas/tap/ocid formula valid"

# Install the *working tree* through Homebrew for local testing (no tag or
# commit needed). Homebrew 6 refuses loose formula files, so this keeps a
# private local tap `safonas/local` (created on demand, never pushed) whose
# `ocid` formula points at a tarball of the checkout in .dev via file://.
# Replaces any installed `ocid` (from the real tap); `just brew` swaps back.
[no-cd]
brew-local:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{ justfile_directory() }}"
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
