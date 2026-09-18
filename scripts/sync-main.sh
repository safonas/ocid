#!/usr/bin/env bash
# Sync local main with github/main, tolerating squash-merged PRs.
# Used by `just sync` and `just publish-release` (which must check out the
# merged main); `cut-release` doesn't need it — it branches straight off
# the fetched github/main tip and never touches local main.
#
# A plain `git pull --ff-only` breaks after every squash merge: local main
# then holds the PR's individual commits while the remote holds the squashed
# one (identical tree, different history). This script:
#   - fast-forwards when local is behind,
#   - resets to the remote when the trees are identical (a squash merge
#     landed upstream), reporting how many local commits it drops (they
#     remain on their release/* or feature branch and in the reflog),
#   - refuses otherwise (genuinely diverged or unpushed work).
set -euo pipefail

remote="${1:-github}"
branch="${2:-main}"

git diff --quiet && git diff --cached --quiet || { echo "error: working tree has uncommitted changes" >&2; exit 1; }
git checkout -q "$branch"
git fetch -q "$remote" "$branch"

if git merge-base --is-ancestor "$branch" "$remote/$branch"; then
    # Local is an ancestor of the remote: plain fast-forward.
    git merge --ff-only "$remote/$branch"
elif git diff --quiet "$branch" "$remote/$branch"; then
    # Same tree, different history: a squash merge landed upstream. Resetting
    # only drops local duplicates of what the remote already contains.
    dropped=$(git rev-list --count "$remote/$branch..$branch")
    echo "note: squash merge detected; resetting $branch to $remote/$branch (dropping $dropped local commit(s) with identical tree)"
    git reset --hard "$remote/$branch"
else
    echo "error: $branch and $remote/$branch have diverged with different content" >&2
    echo "       rebase onto $remote/$branch or open a PR for the local commits" >&2
    exit 1
fi
git log --oneline -3
