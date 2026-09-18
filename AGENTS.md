# AGENTS.md — context for LLM agents working on ocid

## What is ocid?

A Radicle-style, local-first, peer-to-peer distribution system for OCI container
images. An embedded OCI v2 registry listens on `127.0.0.1:5050` so standard
tooling (`podman`, `docker`, `crane`, `oras`) works unchanged. Behind it,
iroh (QUIC, hole-punching, relays, mDNS) distributes content-addressed blobs
and signed release records.

## Workspace layout

- `crates/ocid-core`: library crate. Identity (`Ed25519` keypair = iroh `EndpointId` = publisher id), release signing/verification, OCI types, retention policy windows, on-disk index, API DTOs.
- `crates/ocid`: the daemon binary (`ocid`). Runs iroh endpoint, gossip, blob store, OCI registry + `/_ocid/*` control API + `/metrics` OpenMetrics.
- `crates/ocictl`: the CLI binary (`ocictl`). Interacts with the daemon over HTTP, manages `policy.toml`, supports offline inspection (`ls`, `whoami`, `policy`).
- `crates/ocitop`: the TUI monitor binary (`ocitop`). Interactive terminal dashboard for daemon status, releases, peers, and real-time SSE events.
- `extensions/podman-desktop`: the Podman Desktop extension (TypeScript + Svelte). Dashboard over the same `/_ocid` API; build via `just ext-*` recipes (node runs in a container, like cargo).
- `scripts/e2e.sh`: multi-node end-to-end integration test runner.
- `docs/DESIGN.md`: architectural design, data models, sequences, and trust boundaries.

## Build and verification environment

The host is an ostree-based Fedora with rootless Podman and SELinux enforcing.
**Do not run cargo/rustc directly on the host.** Always use `just` recipes, which
execute inside a container with cached volumes and shared `:z` SELinux bind mounts.
Builder images are Wolfi-based (Chainguard) and pinned by digest: rust + node for
compilation, wolfi-base for the runtime image. Do not use `/tmp` for scratch work;
keep temporary files inside the workspace (e.g. `./.dev/`) so they stay on the same
filesystem and SELinux context.

### Essential commands

- `just check`: run `cargo check` inside container
- `just test`: run workspace unit tests (`cargo test`)
- `just clippy`: run lints with warnings-as-errors (`cargo clippy --all-targets -- -D warnings`)
- `just fmt`: format Rust code (`cargo fmt`)
- `just fmt-check`: verify Rust formatting without modifying files
- `just bin`: build and copy debug binaries to `./bin/{ocid,ocictl,ocitop}`
- `just e2e`: build binaries and run full end-to-end test suite (`scripts/e2e.sh`)
- `just ci`: run `fmt-check` + `clippy` + `test` + `e2e` (all CI checks)
- `just hooks`: install git pre-commit and pre-push hooks
- `just brew`: audit the Homebrew tap formula syntax
- `just cut-release <version>`: validate CI, bump version, push release branch, open PR (merge manually)
- `just publish-release <version>`: tag the merge, update the Homebrew tap, open a draft GitHub release that CI publishes with artifacts + provenance (after the PR merged)

### Git hooks

Managed by `.pre-commit-config.yaml`.
- Pre-commit: whitespace, YAML/TOML/JSON sanity, large file check, shellcheck, `just fmt-check`, `just clippy`.
- Pre-push: `just test`.

### Versioning and tagging

- The workspace version lives in the root `Cargo.toml` (`Cargo.lock` follows via `just check`).
- Before creating a **major or minor** version tag, always ask first and suggest the next version — do not tag unilaterally.
- **Patch** tags are fine without asking, provided the changes are miniscule (docs, comments, tiny fixes, digest pins).

## Model routing & cost guidelines

- When configuring agents or invoking sub-tasks, default to cheaper/faster models for repetitive tasks (e.g. searching, file exploration, formatting, simple edits, reading error messages).
- Reserve higher-capability reasoning models for complex architectural design, debugging tricky concurrency/p2p race conditions, and protocol changes.
