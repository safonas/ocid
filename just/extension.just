# Podman Desktop extension submodule: everything under extensions/podman-desktop.
#
#   just ext install   npm install (creates package-lock.json)
#   just ext build     build the extension (backend dist/ + webview media/)
#   just ext check     typecheck (tsc + svelte-check)
#   just ext smoke     run the extension client against a throwaway daemon
#   just ext image     build the extension OCI artifact (scratch, for the catalog)
#   just ext clean     remove build artifacts and dependencies
#
# Invoke from the repo root; `[no-cd]` keeps each recipe running there too
# (otherwise just would cd into `just/`). Self-contained: just modules
# cannot see the parent's variables/recipes, so settings mirror the root
# justfile. The root file provides flat `just ext-*` shims.

set shell := ["bash", "-euo", "pipefail", "-c"]
set minimum-version := "1.47.0"
set lazy

podman := require(env("PODMAN", "podman"))
# Wolfi-based node builder, pinned by digest (override via NODE_IMAGE env).
node_image := env("NODE_IMAGE", "cgr.dev/chainguard/node@sha256:2a2df3a1f79cfe63317e3e6e2b7394cac434648fdfc82e4f702d0e6dc4d9a852")
ext_image := env("EXT_IMAGE", "localhost/ocid-extension:dev")
ext_dir := "extensions/podman-desktop"
# Chainguard images run as a non-root uid by default; map the invoking user
# in (portable across rootless and rootful podman with --userns=keep-id).
host_uid := `id -u`
host_gid := `id -g`
# Named volume for npm's download cache, so `install` after an `ext-clean`
# (or a lockfile change) is served from disk instead of the network.
vol_npm := "ocid-npm-cache"

# Ensure named cache volumes exist.
[private]
volumes:
    @{{ podman }} volume exists {{ vol_npm }} || {{ podman }} volume create {{ vol_npm }} >/dev/null

# Common node-container invocation. `--entrypoint ""` clears the image's
# node entrypoint so the recipe's command runs directly; HOME=/tmp keeps
# npm's cache writable under --userns=keep-id, and the cache itself is in
# the named volume so downloads survive across invocations. NOTE: never
# mount the cache volume with `:Z` — that stamps it with one container's
# private SELinux category and blocks every later container; unlabeled
# mounts keep the volume shared (`container_file_t:s0`), like the cargo
# registry volume.
_node := podman + " run --rm --userns=keep-id --user " + host_uid + ":" + host_gid + ' --entrypoint ""' + " -e HOME=/tmp -v " + vol_npm + ":/tmp/.npm -v " + justfile_directory() + ":/src:Z -w /src/" + ext_dir + " " + node_image

# Same, but with host networking: for scripts that talk to a daemon on the
# host's loopback (127.0.0.1).
_node_net := podman + " run --rm --userns=keep-id --user " + host_uid + ":" + host_gid + ' --entrypoint ""' + " -e HOME=/tmp --network=host -v " + vol_npm + ":/tmp/.npm -v " + justfile_directory() + ":/src:Z -w /src/" + ext_dir + " " + node_image

# Install extension dependencies (creates package-lock.json). The npm
# download cache lives in a named volume, so reinstalls (after `just
# ext-clean`, or on lockfile changes) do not redownload from the network.
[no-cd]
install: volumes
    {{ _node }} npm install --no-fund --no-audit

# Rebuild backend dist/ and webview media/ on every change (Podman Desktop
# watches the extension folder and reloads). Two watchers: backend + webview.
[no-cd]
watch: volumes
    {{ _node }} npm run watch

# Build the extension: backend dist/ + webview media/.
[no-cd]
build:
    {{ _node }} npm run build

# Typecheck the extension: tsc + svelte-check.
[no-cd]
check:
    {{ _node }} npm run typecheck

# Functional smoke test: run the extension client (OcidClient + SseWatcher)
# against a throwaway daemon on 127.0.0.1:5070. Needs ./bin/ocid — the root
# `ext-smoke` shim builds it first via `just bin`.
[no-cd]
smoke:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{ justfile_directory() }}"
    rm -rf .dev/ext-smoke-home
    mkdir -p .dev/ext-smoke-home
    OCID_HOME="$PWD/.dev/ext-smoke-home" ./bin/ocid --listen 127.0.0.1:5070 --no-relay >/dev/null 2>&1 &
    pid=$!
    trap 'kill $pid 2>/dev/null || true; wait $pid 2>/dev/null || true' EXIT
    for _ in $(seq 1 50); do
        curl -sf http://127.0.0.1:5070/_ocid/status >/dev/null && break
        sleep 0.2
    done
    {{ _node_net }} node --experimental-strip-types scripts/smoke.mts

# Build the extension OCI artifact (scratch base, per the publishing guide:
# https://podman-desktop.io/docs/extensions/publish). Push it to a registry
# and register it in the podman-desktop-catalog to distribute.
[no-cd]
image:
    {{ podman }} build -t {{ ext_image }} -f {{ ext_dir }}/Containerfile {{ ext_dir }}

# Remove extension build artifacts and dependencies.
[no-cd]
clean:
    rm -rf {{ ext_dir }}/node_modules {{ ext_dir }}/dist {{ ext_dir }}/media
