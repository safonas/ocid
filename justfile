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
[group('package')]
image:
    {{ podman }} build -t {{ image }} -f Containerfile .

nfpm_image := "ghcr.io/goreleaser/nfpm:v2.47.0@sha256:a662cb167d7b6d3a83920c83d76b12d02b8ac5dd2c13e5c62c15270b23f6df0c"

# Host -> nfpm arch mapping, resolved by just (not the shell).
nfpm_arch := if arch() == "x86_64" { "amd64" } else if arch() == "aarch64" { "arm64" } else { error("unsupported arch: " + arch()) }

# Android arm64 (Termux) cross-build lives in the `android` submodule.
[group('package')]
mod android

# Shim: `just android build`.
[group('package')]
build-android: android::build
# Shim: `just android pkg`.
[group('package')]
pkg-android: android::pkg

# Build .deb + .rpm for the host architecture into ./dist (needs release binaries).
[group('package')]
pkg: release
    #!/usr/bin/env bash
    set -euo pipefail
    ver=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
    stage="dist/stage"
    cfg="dist/nfpm-{{ nfpm_arch }}.yaml"
    mkdir -p "$stage"
    {{ podman }} run --rm --userns=keep-id \
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
[group('package')]
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
[group('package')]
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

# Phase 1 of a release: validate CI, bump the version, push a release
# branch and open a PR. STOPS there: merge the PR manually, then run
# `just publish-release VER` to tag and publish.
[confirm("Cut release branch + PR? This runs full CI, commits the version bump and pushes.")]
[group('release')]
cut-release VERSION:
    #!/usr/bin/env bash
    set -euo pipefail
    ver="{{ VERSION }}"
    ver="${ver#v}"
    [[ "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "error: invalid semantic version '{{ VERSION }}'" >&2; exit 1; }
    tag="v$ver"
    branch="release/$ver"
    [ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || { echo "error: must be on main branch" >&2; exit 1; }
    git fetch -q github
    [ "$(git rev-parse HEAD)" = "$(git rev-parse github/main)" ] || { echo "error: local main differs from github/main; pull first" >&2; exit 1; }
    git diff --quiet && git diff --cached --quiet || { echo "error: working tree has uncommitted changes" >&2; exit 1; }
    git rev-parse "$tag" >/dev/null 2>&1 && { echo "error: tag '$tag' already exists locally" >&2; exit 1; } || true
    git ls-remote --tags github "refs/tags/$tag" | grep -q "$tag" && { echo "error: tag '$tag' already exists on remote" >&2; exit 1; } || true
    git rev-parse --verify --quiet "refs/heads/$branch" >/dev/null && { echo "error: branch '$branch' already exists locally" >&2; exit 1; } || true
    just ci
    sed -i -e "s|^version = \".*\"|version = \"$ver\"|" Cargo.toml
    just check
    git add Cargo.toml Cargo.lock
    git commit -m "chore(release): bump version to $tag"
    git checkout -b "$branch"
    git push -u github "$branch"
    gh pr create --base main --head "$branch" --title "Release $tag" \
        --body "Version bump to $tag. After merging, run \`just publish-release $ver\` to tag and publish."
    echo "-> merge the PR, then run: just publish-release $ver"

# Phase 2 of a release: tag the merged bump, create the GitHub release
# (fires the SLSA build) and update the Homebrew tap. Run on main AFTER
# the cut-release PR has been merged.
[confirm("Publish the release? This tags, pushes the tag, creates a GitHub release and updates the tap.")]
[group('release')]
publish-release VERSION:
    #!/usr/bin/env bash
    set -euo pipefail
    ver="{{ VERSION }}"
    ver="${ver#v}"
    [[ "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "error: invalid semantic version '{{ VERSION }}'" >&2; exit 1; }
    tag="v$ver"
    [ "$(git rev-parse --abbrev-ref HEAD)" = "main" ] || { echo "error: must be on main branch" >&2; exit 1; }
    git diff --quiet && git diff --cached --quiet || { echo "error: working tree has uncommitted changes" >&2; exit 1; }
    git fetch -q github
    [ "$(git rev-parse HEAD)" = "$(git rev-parse github/main)" ] || { echo "error: local main differs from github/main; pull the merged release PR first" >&2; exit 1; }
    grep -q "^version = \"$ver\"$" Cargo.toml || { echo "error: Cargo.toml is not at $ver; has the release PR merged?" >&2; exit 1; }
    git rev-parse "$tag" >/dev/null 2>&1 && { echo "error: tag '$tag' already exists locally" >&2; exit 1; } || true
    git ls-remote --tags github "refs/tags/$tag" | grep -q "$tag" && { echo "error: tag '$tag' already exists on remote" >&2; exit 1; } || true
    git tag -a "$tag" -m "Release $tag"
    git push github "$tag"
    if git remote get-url rad >/dev/null 2>&1; then
        git push rad main "$tag" || echo "warning: push to rad remote failed" >&2
    else
        echo "warning: no rad remote configured, skipping Radicle mirror" >&2
    fi
    archive_url="https://github.com/safonas/ocid/archive/refs/tags/${tag}.tar.gz"
    echo "Waiting for tag archive on GitHub..."
    for _ in $(seq 1 30); do
        if [ "$(curl -sSL -o /dev/null -w '%{http_code}' "$archive_url")" = "200" ]; then
            break
        fi
        sleep 2
    done
    archive_sha=$(curl -sSL --retry 3 "$archive_url" | sha256sum | cut -d' ' -f1)
    echo "Archive SHA256: $archive_sha"
    if command -v brew >/dev/null && brew tap safonas/tap >/dev/null 2>&1; then
        tap=$(brew --repository safonas/tap)
        git -C "$tap" fetch -q origin && git -C "$tap" checkout -q main && git -C "$tap" reset -q --hard origin/main
    else
        tap=".dev/homebrew-tap"
        mkdir -p .dev
        if [ -d "$tap/.git" ]; then
            git -C "$tap" fetch -q origin && git -C "$tap" checkout -q main && git -C "$tap" reset -q --hard origin/main
        else
            git clone -q git@github.com:safonas/homebrew-tap.git "$tap"
        fi
    fi
    tap_origin=$(git -C "$tap" remote get-url origin 2>/dev/null || true)
    case "$tap_origin" in
        https://github.com/*) git -C "$tap" remote set-url origin "git@github.com:${tap_origin#https://github.com/}" ;;
    esac
    formula="$tap/Formula/ocid.rb"
    sed -i -e "s|^  url .*|  url \"$archive_url\"|" -e "s|^  sha256 .*|  sha256 \"$archive_sha\"|" "$formula"
    if ! grep -q 'crates/ocitop' "$formula"; then
        sed -i -e '/crates\/ocictl/a\    system "cargo", "install", *std_cargo_args(path: "crates/ocitop")' "$formula"
    fi
    if command -v brew >/dev/null; then
        echo "Auditing formula..."
        brew audit --formula safonas/tap/ocid
    fi
    git -C "$tap" commit -am "ocid: bump to $tag"
    git -C "$tap" push origin main
    echo "Homebrew tap updated."
    gh release create "$tag" --target main --title "$tag" --generate-notes
    echo "Release URL: https://github.com/safonas/ocid/releases/tag/$tag"
    gh run list --limit 3

# Alias for publish-release (confirmation happens there).
[group('release')]
publish VERSION:
    just publish-release {{ VERSION }}

# Remove target volume and ./bin.
[confirm("Delete the target volume, ./bin and ./dist?")]
[group('dev')]
clean:
    -{{ podman }} volume rm {{ vol_target }}
    rm -rf bin dist
