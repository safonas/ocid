# Release submodule: two-phase, PR-based releases (`main` is protected).
#
#   just ship cut-release VER      validate CI, bump, push branch, open PR, STOP
#   just ship publish-release VER  tag the merge, GH release, tap update
#   just ship publish VER           alias for publish-release
#
# Invoke from the repo root; `[no-cd]` keeps each recipe running there too
# (otherwise just would cd into `just/`). Only git/gh/curl are needed.

set shell := ["bash", "-euo", "pipefail", "-c"]
set minimum-version := "1.47.0"

# Phase 1 of a release: validate CI, bump the version, push a release
# branch and open a PR. STOPS there: merge the PR manually, then run
# `just publish-release VER` to tag and publish.
[no-cd]
[confirm("Cut release branch + PR? This runs full CI, commits the version bump and pushes.")]
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
[no-cd]
[confirm("Publish the release? This tags, pushes the tag, creates a GitHub release and updates the tap.")]
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
[no-cd]
publish VERSION:
    just ship::publish-release {{ VERSION }}
