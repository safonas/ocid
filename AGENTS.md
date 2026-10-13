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
- The SLSA generator workflow (`slsa.yml`) must be called by **tag ref**, never a SHA: it resolves its
  SLSA-verified builder binary by the ref it is called with and rejects commit SHAs (upstream
  `RENOVATE.md` documents this; `renovate.json` exempts it from digest pinning). Consequence:
  OpenSSF Scorecard files a "Pinned-Dependencies" alert for it — that alert is expected and should
  be dismissed as *won't fix* with a reference to this note.

### Renovate dependency updates

Renovate runs weekly (Mondays before 6am UTC) and opens PRs; automerge is
disabled by config, so PRs must be merged manually — including by agents.
Find open ones with `gh pr list --author app/renovate --state open`.

- **Merge strictly one PR at a time.** Every merge to `main` invalidates the
  other open PRs' required checks, forcing full re-runs. Typical durations:
  CI (fmt + clippy + test) ~1–2 min, OpenSSF Scorecard ~20 s, CodeQL ~6 min,
  Trivy ~8–10 min.
- **Merging:** a ruleset on `main` blocks merge commits and requires status
  checks. Poll `gh pr checks <n>` until it exits 0 (its Scorecard line shows
  "skipping" on PRs — that variant only runs on `main`), then merge with
  `gh pr merge <n> --rebase --admin`. If the API refuses with a ruleset or
  OAuth workflow-scope error before checks finish, wait and retry. After
  each merge: `git pull github main`, then move to the next PR.
- **Grouped PRs** (e.g. "cargo minor and patch") bundle many bumps into one
  merge; `Cargo.lock` changes additionally trigger the `cargo audit` check.
- **Digest pinning** (`pinDigests: true`) is enforced. Do not accept action
  updates that drop digest pins, and avoid adopting actions that publish no
  version tags — their pin would freeze forever (this bit
  `dtolnay/rust-toolchain`, since replaced with plain rustup + the repo's
  `rust-toolchain.toml`). The SLSA generator is the only deliberate
  exemption (see above).

### Cutting a release

Two-phase, PR-based (main is protected); full details live in `just/ship.just`,
but this section is self-sufficient. Both recipes prompt for confirmation —
pipe `yes |` when running non-interactively.

**Phase 1 — `just cut-release <ver>`** (e.g. `0.9.0-rc.1`; leading `v` optional):

- Enforces a clean tree, `github` remote, and that `v<ver>` / `release/<ver>` don't exist.
- Cuts `release/<ver>` from the fetched `github/main` tip — local `main` is
  never touched, so it may run from any clean worktree. Note it switches the
  invoking worktree onto the release branch before CI; don't switch branches
  in that worktree until it finishes.
- Runs `just ci`, bumps the version in `Cargo.toml` + `Cargo.lock` +
  `extensions/podman-desktop/package.json` + its `package-lock.json` (only
  these four files are committed), pushes, opens the "Release v<ver>" PR, and
  STOPS.
- Merge the PR with `gh pr merge --rebase` — a ruleset blocks merge commits
  on main.

**Phase 2 — `just publish-release <ver>`** (after the PR merges):

- Must run from a checkout where local `main` lives (it checks out and
  ff-pulls main; in a worktree setup that is the primary checkout — linked
  worktrees cannot check main out).
- Tags `v<ver>` on main, pushes the tag to `github` and `rad`, creates a
  DRAFT GitHub release, dispatches `slsa.yml` (workflow from main, build
  from the tag), and watches it: CI attaches binaries, .deb/.rpm and SLSA
  provenance, then publishes the release as its last step.
- Pre-release versions (any `-suffix`, e.g. `-rc.1`, `-alpha.2`) skip the
  Homebrew tap and carry the GitHub prerelease flag; full versions also
  update the tap (formula template `packaging/brew/ocid.rb`).
- Idempotent/resumable: safe to re-run after a mid-flight failure. Recovery
  recipes: `just republish <ver>` (re-run SLSA against the existing draft),
  `just republish-image <ver>` (rebuild the extension OCI image,
  ghcr.io/safonas/ocid-extension:<tag> + `:testing`).

## Model routing & cost guidelines

- When configuring agents or invoking sub-tasks, default to cheaper/faster models for repetitive tasks (e.g. searching, file exploration, formatting, simple edits, reading error messages).
- Reserve higher-capability reasoning models for complex architectural design, debugging tricky concurrency/p2p race conditions, and protocol changes.
