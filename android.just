# Android arm64 (Termux) cross-build submodule.
#
#   just android build   cross-compile release binaries (NDK r27d, API 26)
#   just android pkg     package them into ./dist/ocid-android-arm64.tar.gz
#
# Invoke from the repo root; `justfile_directory()` still points there.
# This module is self-contained: just modules cannot see the parent's
# variables/recipes, so podman/image/volume settings mirror the root justfile.

set shell := ["bash", "-euo", "pipefail", "-c"]
set minimum-version := "1.47.0"

podman := require(env("PODMAN", "podman"))
# Builder image pinned by digest for reproducible builds (override via RUST_IMAGE env).
rust_image := env("RUST_IMAGE", "docker.io/library/rust:1.98.1-slim-bookworm@sha256:ebd900bae66fd508b466cef82d64a83a5fb34682e4c8b2797a42908bddc95a57")
project := "ocid"
vol_registry := project + "-cargo-registry"
vol_target := project + "-target"

# Android NDK for Termux (aarch64-linux-android, bionic libc).
# Pinned LTS; API 26 keeps compat back to Android 8 while running fine on
# current devices (Android 13+). The NDK zip (~700MB) is cached in a named volume
# so re-builds don't re-download.
android_target := "aarch64-linux-android"
ndk_version := "r27d"
ndk_api := "26"
vol_ndk := project + "-android-ndk"

# Cross-compile release binaries for Android arm64 (Termux) into the target volume.
build:
    #!/usr/bin/env bash
    set -euo pipefail
    {{podman}} volume exists {{vol_registry}} || {{podman}} volume create {{vol_registry}} >/dev/null
    {{podman}} volume exists {{vol_target}} || {{podman}} volume create {{vol_target}} >/dev/null
    {{podman}} volume exists {{vol_ndk}} || {{podman}} volume create {{vol_ndk}} >/dev/null
    {{podman}} run --rm \
        -v {{vol_ndk}}:/opt/android-ndk \
        -w /tmp \
        {{rust_image}} sh -c 'set -e; \
            if [ ! -d /opt/android-ndk/android-ndk-{{ndk_version}} ]; then \
                apt-get update && apt-get install -y --no-install-recommends curl unzip ca-certificates; \
                curl -sSfL https://dl.google.com/android/repository/android-ndk-{{ndk_version}}-linux.zip -o /opt/android-ndk/ndk.zip; \
                unzip -q /opt/android-ndk/ndk.zip -d /opt/android-ndk; \
                rm /opt/android-ndk/ndk.zip; \
            fi'
    {{podman}} run --rm --userns=keep-id \
        -e CARGO_HOME=/usr/local/cargo \
        -e CARGO_TARGET_DIR=/target \
        -e CARGO_TERM_COLOR=always \
        -e RUST_BACKTRACE=1 \
        -v {{justfile_directory()}}:/src:Z \
        -v {{vol_registry}}:/usr/local/cargo/registry \
        -v {{vol_target}}:/target \
        -v {{vol_ndk}}:/opt/android-ndk:ro \
        -w /src \
        {{rust_image}} sh -c 'set -e; \
            export ANDROID_NDK_HOME=/opt/android-ndk/android-ndk-{{ndk_version}}; \
            export TOOLCHAIN=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin; \
            rustup target add {{android_target}}; \
            export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$TOOLCHAIN/aarch64-linux-android{{ndk_api}}-clang; \
            export CC_aarch64_linux_android=$TOOLCHAIN/aarch64-linux-android{{ndk_api}}-clang; \
            export AR_aarch64_linux_android=$TOOLCHAIN/llvm-ar; \
            cargo build --release --locked --target {{android_target}}'

# Package Android arm64 binaries into ./dist (standalone files + tarball, no .deb/.rpm).
pkg: build
    #!/usr/bin/env bash
    set -euo pipefail
    epoch=$(git log -1 --pretty=%ct)
    mkdir -p dist
    {{podman}} run --rm --userns=keep-id \
        -e TAR_EPOCH="$epoch" \
        -v {{vol_target}}:/target \
        -v {{justfile_directory()}}/dist:/out:Z \
        {{rust_image}} sh -c 'cp /target/{{android_target}}/release/ocid /out/ocid-android-arm64 \
            && cp /target/{{android_target}}/release/ocictl /out/ocictl-android-arm64 \
            && cp /target/{{android_target}}/release/ocitop /out/ocitop-android-arm64 \
            && mkdir -p /out/.android-stage \
            && cp /out/ocid-android-arm64 /out/.android-stage/ocid \
            && cp /out/ocictl-android-arm64 /out/.android-stage/ocictl \
            && cp /out/ocitop-android-arm64 /out/.android-stage/ocitop \
            && cd /out && GZIP="-n" tar --sort=name --mtime="@$TAR_EPOCH" --owner=0 --group=0 --numeric-owner -czf ocid-android-arm64.tar.gz -C .android-stage ocid ocictl ocitop \
            && rm -rf /out/.android-stage'
    ls -l dist/ocid-android-arm64.tar.gz dist/ocid-android-arm64 dist/ocictl-android-arm64 dist/ocitop-android-arm64
