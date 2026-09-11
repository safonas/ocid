#!/usr/bin/env bash
# release.sh — automated release and Homebrew tap publication for ocid.
#
# Usage:
#   scripts/release.sh <version>
#   just release <version>
#
# Example:
#   scripts/release.sh 0.3.1
#
# Steps performed:
#   1. Validates working tree is clean and on main branch.
#   2. Runs full CI validation (fmt-check, clippy, unit tests, e2e tests).
#   3. Updates version in Cargo.toml and synchronizes Cargo.lock.
#   4. Commits version bump, creates annotated git tag, and pushes to GitHub.
#   5. Creates GitHub Release (triggers SLSA package build workflow).
#   6. Updates safonas/homebrew-tap formula (url, sha256), validates with brew audit,
#      and pushes tap update so Homebrew drift check passes immediately.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [ $# -ne 1 ]; then
    echo "usage: $0 <version>" >&2
    echo "example: $0 0.3.1" >&2
    exit 1
fi

raw_ver="$1"
ver="${raw_ver#v}"

if [[ ! "$ver" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "error: invalid semantic version '$raw_ver'" >&2
    exit 1
fi

tag="v$ver"

# 1. Pre-flight checks
echo "==> [1/6] Pre-flight checks..."
command -v git >/dev/null || { echo "error: git not found" >&2; exit 1; }
command -v gh >/dev/null || { echo "error: gh CLI not found" >&2; exit 1; }
command -v just >/dev/null || { echo "error: just not found" >&2; exit 1; }

current_branch=$(git rev-parse --abbrev-ref HEAD)
if [ "$current_branch" != "main" ]; then
    echo "error: must be on main branch (currently on '$current_branch')" >&2
    exit 1
fi

if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "error: working directory has uncommitted changes" >&2
    exit 1
fi

if git rev-parse "$tag" >/dev/null 2>&1; then
    echo "error: git tag '$tag' already exists locally" >&2
    exit 1
fi

if git ls-remote --tags github "refs/tags/$tag" | grep -q "$tag"; then
    echo "error: git tag '$tag' already exists on remote 'github'" >&2
    exit 1
fi

# 2. Run CI validation suite
echo "==> [2/6] Running CI checks (fmt, clippy, unit tests, e2e)..."
just ci

# 3. Update version in Cargo.toml & Cargo.lock
echo "==> [3/6] Updating version to $ver..."
sed -i -e "s|^version = \".*\"|version = \"$ver\"|" Cargo.toml
just check
git add Cargo.toml Cargo.lock

# 4. Commit and push
echo "==> [4/6] Committing and tagging $tag..."
git commit -m "chore(release): bump version to $tag"
git push github main
git tag -a "$tag" -m "Release $tag"
git push github "$tag"

# 5. Update and push Homebrew tap (before release event fires drift check)
echo "==> [5/6] Updating Homebrew tap..."
archive_url="https://github.com/safonas/ocid/archive/refs/tags/${tag}.tar.gz"

echo "    Waiting for tag archive on GitHub..."
for _ in $(seq 1 30); do
    if [ "$(curl -sSL -o /dev/null -w '%{http_code}' "$archive_url")" = "200" ]; then
        break
    fi
    sleep 2
done

archive_sha=$(curl -sSL --retry 3 "$archive_url" | sha256sum | cut -d' ' -f1)
echo "    Archive SHA256: $archive_sha"

if command -v brew >/dev/null && brew tap safonas/tap >/dev/null 2>&1; then
    tap=$(brew --repository safonas/tap)
    git -C "$tap" fetch -q origin && git -C "$tap" checkout -q main && git -C "$tap" reset -q --hard origin/main
else
    tap="$ROOT/.dev/homebrew-tap"
    mkdir -p "$ROOT/.dev"
    if [ -d "$tap/.git" ]; then
        git -C "$tap" fetch -q origin && git -C "$tap" checkout -q main && git -C "$tap" reset -q --hard origin/main
    else
        git clone -q git@github.com:safonas/homebrew-tap.git "$tap"
    fi
fi

# Ensure remote is pushable via SSH if https was default
tap_origin=$(git -C "$tap" remote get-url origin 2>/dev/null || true)
if [[ "$tap_origin" =~ ^https://github.com/(.*) ]]; then
    git -C "$tap" remote set-url origin "git@github.com:${BASH_REMATCH[1]}"
fi

formula="$tap/Formula/ocid.rb"
sed -i -e "s|^  url .*|  url \"$archive_url\"|" -e "s|^  sha256 .*|  sha256 \"$archive_sha\"|" "$formula"
if ! grep -q 'crates/ocitop' "$formula"; then
    sed -i -e '/crates\/ocictl/a\    system "cargo", "install", *std_cargo_args(path: "crates/ocitop")' "$formula"
fi

if command -v brew >/dev/null; then
    echo "    Auditing formula..."
    brew audit --formula safonas/tap/ocid
fi

git -C "$tap" commit -am "ocid: bump to $tag"
git -C "$tap" push origin main
echo "    Homebrew tap updated and pushed to origin/main."

# 6. Create GitHub release (triggers SLSA generic generator and drift check)
echo "==> [6/6] Creating GitHub release for $tag..."
gh release create "$tag" --target main --title "$tag" --generate-notes

echo ""
echo "Release $tag complete!"
echo "Release URL: https://github.com/safonas/ocid/releases/tag/$tag"
echo "Workflows triggered:"
gh run list --limit 3
