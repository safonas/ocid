# Development

Everything builds inside podman; only `podman` and `just` are needed on the host.
All `just` recipes run in a pinned builder container
(`docker.io/library/rust:1.98.1-slim-bookworm@sha256:ebd900ba…`) with cached
cargo registry/target volumes and proper `:Z` SELinux bind mounts — never run
`cargo`/`rustc` directly on the host.

## Recipes

```sh
just             # list recipes
just build       # incremental debug build (cached registry + target volumes)
just release     # optimized build
just check       # cargo check in the build container
just test        # unit tests
just clippy      # lints (warnings are errors, same as CI)
just fmt         # rustfmt
just fmt-check   # fail if sources are not formatted
just bin         # debug build -> ./bin/{ocid,ocictl}
just run ARGS    # run the ocid daemon in the builder (state in ./.dev/ocid-home)
just ctl ARGS    # run ocictl in the builder against the same state
just shell       # interactive shell in the build container
just e2e         # build, then scripts/e2e.sh: two daemons on this host, driven by podman/curl/ocictl
just ci          # fmt --check, clippy -D warnings, tests, e2e
just image       # runtime image: podman build -> localhost/ocid:dev
just pkg         # .deb + .rpm for the host arch into ./dist (via pinned nfpm)
just brew        # verify the Homebrew tap formula (checkout in .dev, install from local file)
just hooks       # install git hooks via pre-commit (fmt/clippy on commit, tests on push)
just clean       # remove target volume, ./bin, ./dist
```

## End-to-end tests

`scripts/e2e.sh` needs `podman`, `curl`, `jq` on the host (`oras` enables the
referrers check). It covers publish → replicate → run, on-demand pull,
Range/DELETE/metrics, follow/seed/pin windows with pruning, aliases, GC and
offline `ocictl`.

## Runtime container image

```sh
podman run -d --name ocid -p 127.0.0.1:5050:5050 -v ocid-data:/data localhost/ocid:dev
podman exec ocid ocictl status
```

The `Containerfile` is a multi-stage build: the pinned Rust builder compiles
release binaries with `SOURCE_DATE_EPOCH` and path remapping for bit-for-bit
reproducible output, then the pinned `debian:bookworm-slim` runtime stage
copies in just the two binaries and runs as the `ocid` user.

## Native packages

`just pkg` builds `.deb` + `.rpm` for the host architecture via
[nfpm](https://github.com/goreleaser/nfpm) (also digest-pinned) from
`packaging/nfpm.yaml`, including the systemd unit
(`packaging/systemd/ocid.service`) and maintainer scripts
(`packaging/scripts/`). Releases build both `amd64` and `arm64` packages in CI
(see `.github/workflows/slsa.yml`).

## Homebrew tap

Releases are distributed via `safonas/homebrew-tap` (`brew install
safonas/tap/ocid`). `just brew` verifies the formula without touching
system-wide `/tmp`: it syncs the tap into `.dev/homebrew-tap` (reset to
`origin/main` on every run), bumps `url`/`sha256` to the current `Cargo.toml`
version (the `v<ver>` tag must already be pushed to GitHub), installs from the
local formula file, and smoke-tests both binaries. If the formula changed,
commit and push it from the checkout:

```sh
git -C .dev/homebrew-tap commit -am "ocid: bump to vX.Y.Z" && git -C .dev/homebrew-tap push
```

A release-triggered CI job (`.github/workflows/brew-drift.yml`, also manually
dispatchable) asserts the tap formula's `url`/`sha256` match the latest GitHub
release and fails loudly on drift. It is check-only — no full `brew install`
build in CI (that duplicates the SLSA release build); the real build test is
`just brew` locally before pushing the tap.

## Git hooks

Managed by `.pre-commit-config.yaml` (install with `just hooks`):

- Pre-commit: whitespace, YAML/TOML/JSON sanity, large file check, shellcheck,
  `just fmt-check`, `just clippy`.
- Pre-push: `just test`.
