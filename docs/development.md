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
just bin         # debug build -> ./bin/{ocid,ocictl,ocitop}
just run ARGS    # run the ocid daemon in the builder (state in ./.dev/ocid-home)
just ctl ARGS    # run ocictl in the builder against the same state
just shell       # interactive shell in the build container
just e2e         # build, then scripts/e2e.sh: two daemons on this host, driven by podman/curl/ocictl
just ci          # fmt --check, clippy -D warnings, tests, e2e
just image       # runtime image: podman build -> localhost/ocid:dev
just pkg         # .deb + .rpm for the host arch into ./dist (via pinned nfpm)
just brew        # verify Homebrew tap formula syntax (brew audit)
just brew-local  # test-install working tree through a local private tap
just publish-release VER # automate release: CI checks, bump, tag, GH release, tap update
just hooks       # install git hooks via pre-commit (fmt/clippy on commit, tests on push)
just clean       # remove target volume, ./bin, ./dist
```

## End-to-end tests

`scripts/e2e.sh` needs `podman`, `curl`, `jq` on the host (`oras` enables the
referrers check). It covers publish → replicate → run, on-demand pull,
Range/DELETE/metrics, follow/seed/pin windows with pruning, aliases, GC and
offline `ocictl`.

## Security scanning

Vulnerability scanning runs in CI (`.github/workflows/security.yml`): Trivy
scans the container image and working tree, and OpenSSF Scorecard reviews the
repository. See [SECURITY.md](../SECURITY.md) for reporting and local scan
commands.

## Runtime container image

```sh
podman run -d --name ocid -p 127.0.0.1:5050:5050 -v ocid-data:/data localhost/ocid:dev
podman exec ocid ocictl status
```

The `Containerfile` is a multi-stage build: the pinned Rust builder compiles
release binaries with `SOURCE_DATE_EPOCH` and path remapping for bit-for-bit
reproducible output, then the pinned Wolfi (`wolfi-base`) runtime stage
copies in just the three binaries and runs as the `ocid` user. Wolfi is a
rolling base, so re-pin `RUNTIME_IMAGE` regularly to pick up fresh fixes.

## Native packages

`just pkg` builds `.deb` + `.rpm` for the host architecture via
[nfpm](https://github.com/goreleaser/nfpm) (also digest-pinned) from
`packaging/nfpm.yaml`, including the systemd unit
(`packaging/systemd/ocid.service`) and maintainer scripts
(`packaging/scripts/`). Releases build both `amd64` and `arm64` packages in CI
(see `.github/workflows/slsa.yml`).

## Homebrew tap & releases

Releases are distributed via `safonas/homebrew-tap` (`brew install safonas/tap/ocid`).

To cut a new release end-to-end:

```sh
just publish-release <version>  # or scripts/release.sh <version>
```

This script:
1. Verifies the working tree is clean and runs `just ci` (all lints and tests).
2. Bumps `Cargo.toml` and `Cargo.lock`.
3. Commits, tags (`v<version>`), and pushes to GitHub.
4. Creates the GitHub release (triggering SLSA package and provenance builds).
5. Updates `Formula/ocid.rb` in `safonas/homebrew-tap`, audits with `brew audit`, and pushes the tap.

A release-triggered CI job (`.github/workflows/brew-drift.yml`) asserts that the tap formula tracks the latest release and stays green.

To audit the formula syntax without cutting a release:
```sh
just brew
```

To test building and installing your current working tree through Homebrew locally:
```sh
just brew-local
```

## Git hooks

Managed by `.pre-commit-config.yaml` (install with `just hooks`):

- Pre-commit: whitespace, YAML/TOML/JSON sanity, large file check, shellcheck,
  `just fmt-check`, `just clippy`.
- Pre-push: `just test`.
