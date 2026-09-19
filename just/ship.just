# Release submodule: two-phase, PR-based releases (`main` is protected).
#
#   just ship cut-release VER      validate CI, bump, push branch, open PR, STOP
#   just ship publish-release VER  tag the merge, tap update, draft release + CI publish
#   just ship republish VER        re-dispatch the release build for an existing draft
#   just ship publish VER           alias for publish-release
#
# Invoke from the repo root; `[no-cd]` keeps each recipe running there too
# (otherwise just would cd into `just/`). Only git/gh/curl are needed.
#
# All recipes need a `github` remote and fail fast without one (guard
# below). `publish-release` is resumable: every step is idempotent, so
# after a mid-flight failure (tap push, draft creation, failed CI run)
# the same invocation settles the remaining work instead of erroring out
# on the already-existing tag/release.

# Fail fast when the `github` remote the release recipes fetch/push is
# missing — a clone with only `origin` would otherwise burn a full CI
# run in cut-release before git fails on 'github'.
[private]
[no-cd]
github-remote:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! git remote get-url github >/dev/null 2>&1; then
        echo "error: no 'github' remote; the release recipes fetch and push it. Fix with:" >&2
        echo "  git remote add github \"\$(git remote get-url origin)\"" >&2
        exit 1
    fi

set shell := ["bash", "-euo", "pipefail", "-c"]
set minimum-version := "1.47.0"

# Phase 1 of a release: validate CI, bump the version, push a release
# branch and open a PR. STOPS there: merge the PR manually, then run
# `just publish-release VER` to tag and publish.
#
# The branch is cut straight from the fetched github/main tip: local main
# is never touched by the release flow.
[confirm("Cut release branch + PR? This runs full CI, commits the version bump and pushes.")]
[no-cd]
cut-release VERSION: github-remote
    #!/usr/bin/env bash
    set -euo pipefail
    ver="{{ VERSION }}"
    ver="${ver#v}"
    [[ "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "error: invalid semantic version '{{ VERSION }}'" >&2; exit 1; }
    tag="v$ver"
    branch="release/$ver"
    git diff --quiet && git diff --cached --quiet || { echo "error: working tree has uncommitted changes" >&2; exit 1; }
    git rev-parse "$tag" >/dev/null 2>&1 && { echo "error: tag '$tag' already exists locally" >&2; exit 1; } || true
    git ls-remote --tags github "refs/tags/$tag" | grep -q "$tag" && { echo "error: tag '$tag' already exists on remote" >&2; exit 1; } || true
    git rev-parse --verify --quiet "refs/heads/$branch" >/dev/null && { echo "error: branch '$branch' already exists locally" >&2; exit 1; } || true
    git fetch -q github main
    git checkout -b "$branch" github/main
    just ci
    sed -i -e "s|^version = \".*\"|version = \"$ver\"|" Cargo.toml
    just check
    git add Cargo.toml Cargo.lock
    git commit -m "chore(release): bump version to $tag"
    git push -u github "$branch"
    gh pr create --base main --head "$branch" --title "Release $tag" \
        --body "Version bump to $tag. After merging, run \`just publish-release $ver\` to tag and publish."
    echo "-> merge the PR, then run: just publish-release $ver"

# Phase 2 of a release: tag the merged bump, update the Homebrew tap,
# open a draft GitHub release and dispatch the SLSA build. CI attaches
# binaries, .deb/.rpm packages and SLSA provenance to the draft, then
# publishes it as its last step (releases are immutable once published,
# so the draft phase is required). Run AFTER the cut-release PR has been
# merged (switches to an updated main itself).
[confirm("Publish the release? This tags, pushes the tag, creates a GitHub release and updates the tap.")]
[no-cd]
publish-release VERSION: github-remote
    #!/usr/bin/env bash
    set -euo pipefail
    ver="{{ VERSION }}"
    ver="${ver#v}"
    [[ "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "error: invalid semantic version '{{ VERSION }}'" >&2; exit 1; }
    tag="v$ver"
    git diff --quiet && git diff --cached --quiet || { echo "error: working tree has uncommitted changes" >&2; exit 1; }
    git checkout -q main
    git pull github main --ff-only
    grep -q "^version = \"$ver\"$" Cargo.toml || { echo "error: Cargo.toml is not at $ver; has the release PR merged?" >&2; exit 1; }
    if gh release view "$tag" >/dev/null 2>&1 \
        && [ "$(gh release view "$tag" --json isDraft --jq .isDraft)" != "true" ]; then
        echo "error: release '$tag' is already published and immutable; nothing to do" >&2
        exit 1
    fi
    # Resumable: reuse a tag left by an earlier attempt, but only when it
    # points at current main (the release PR merge).
    if ! git rev-parse --verify --quiet "$tag" >/dev/null 2>&1; then
        git fetch -q github "refs/tags/$tag" 2>/dev/null || true
    fi
    if git rev-parse --verify --quiet "$tag" >/dev/null 2>&1; then
        if [ "$(git rev-parse "$tag^{commit}")" != "$(git rev-parse HEAD)" ]; then
            echo "error: tag '$tag' exists but does not point at current main; refusing to reuse it" >&2
            exit 1
        fi
        echo "-> tag '$tag' already exists and matches main (resuming)"
    else
        git tag -a "$tag" -m "Release $tag"
    fi
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
    # Update the tap through a private clone under .dev — never invoke the
    # host brew in the release path. Proxied brew setups (brew-proxy
    # elevates via a DBus service and pops an admin-password dialog) and
    # foreign-owned installs (/var/home/linuxbrew: git "dubious ownership",
    # unwritable formula dir) both make host-brew use disruptive or broken.
    # `just brew` audits the live tap on a host with a usable Homebrew.
    tap=".dev/homebrew-tap"
    mkdir -p .dev
    if [ -d "$tap/.git" ]; then
        git -C "$tap" fetch -q origin && git -C "$tap" checkout -q main && git -C "$tap" reset -q --hard origin/main
    else
        git clone -q git@github.com:safonas/homebrew-tap.git "$tap"
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
    if git -C "$tap" diff --quiet; then
        echo "Homebrew tap already at $tag."
    else
        git -C "$tap" commit -am "ocid: bump to $tag"
        git -C "$tap" push origin main
        echo "Homebrew tap updated."
    fi
    if gh release view "$tag" >/dev/null 2>&1; then
        echo "-> draft release for '$tag' already exists (resuming)"
    else
        gh release create "$tag" --draft --target main --title "$tag" --generate-notes
        echo "Draft release: https://github.com/safonas/ocid/releases/tag/$tag"
    fi
    since=$(date -u -d '1 minute ago' +%Y-%m-%dT%H:%M:%SZ)
    gh workflow run slsa.yml --ref main -f tag="$tag"
    echo "Dispatched the SLSA build; watching it (it publishes $tag as its last step)..."
    run_id=""
    for _ in $(seq 1 30); do
        run_id=$(gh run list --workflow slsa.yml --limit 5 --json databaseId,createdAt \
            --jq "[.[] | select(.createdAt >= \"$since\")][0].databaseId // \"\"")
        [ -n "$run_id" ] && break
        sleep 2
    done
    [ -n "$run_id" ] || { echo "error: dispatched slsa.yml run not found; check the Actions page" >&2; exit 1; }
    if ! gh run watch "$run_id" --exit-status --interval 30; then
        echo "error: the SLSA build failed; the draft keeps its assets -- fix the cause and run 'just republish $ver'" >&2
        exit 1
    fi
    echo "Published: https://github.com/safonas/ocid/releases/tag/$tag"

# Retry publishing a release whose CI run failed: re-dispatches the SLSA
# workflow against the existing draft release. Drafts stay mutable under
# immutable releases, so partial uploads are simply clobbered. Builds run
# from the tag, so main having moved on is fine.
[confirm("Re-dispatch the release build?")]
[no-cd]
republish VERSION: github-remote
    #!/usr/bin/env bash
    set -euo pipefail
    ver="{{ VERSION }}"
    ver="${ver#v}"
    [[ "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || { echo "error: invalid semantic version '{{ VERSION }}'" >&2; exit 1; }
    tag="v$ver"
    git ls-remote --tags github "refs/tags/$tag" | grep -q "$tag" || { echo "error: tag '$tag' does not exist on github" >&2; exit 1; }
    if ! gh release view "$tag" >/dev/null 2>&1; then
        echo "error: no GitHub release for '$tag'; run 'just publish-release $ver' instead" >&2
        exit 1
    fi
    if [ "$(gh release view "$tag" --json isDraft --jq .isDraft)" != "true" ]; then
        echo "error: release '$tag' is already published and immutable; assets cannot be changed" >&2
        exit 1
    fi
    since=$(date -u -d '1 minute ago' +%Y-%m-%dT%H:%M:%SZ)
    gh workflow run slsa.yml --ref main -f tag="$tag"
    echo "Re-dispatched the SLSA build for $tag; watching it..."
    run_id=""
    for _ in $(seq 1 30); do
        run_id=$(gh run list --workflow slsa.yml --limit 5 --json databaseId,createdAt \
            --jq "[.[] | select(.createdAt >= \"$since\")][0].databaseId // \"\"")
        [ -n "$run_id" ] && break
        sleep 2
    done
    [ -n "$run_id" ] || { echo "error: dispatched slsa.yml run not found; check the Actions page" >&2; exit 1; }
    if ! gh run watch "$run_id" --exit-status --interval 30; then
        echo "error: the SLSA build failed again; the draft keeps its assets" >&2
        exit 1
    fi
    echo "Published: https://github.com/safonas/ocid/releases/tag/$tag"

# Alias for publish-release (confirmation happens there).
[no-cd]
publish VERSION:
    just ship::publish-release {{ VERSION }}
