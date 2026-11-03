#!/usr/bin/env bash
# Merge open Renovate PRs on safonas/ocid, one by one, rebasing as needed.
#
# Why sequential? The `main` ruleset enforces:
#   - strict required status checks (branch must be up to date with main), and
#   - required linear history (rebase merge, no merge commits).
# So after every merge all remaining PR branches go stale and must be rebased
# onto the new main, force-pushed, and re-tested before the next merge.
#
# For each PR the script:
#   1. rebases the branch onto origin/main (conflicts resolve to the PR's
#      version, correct for renovate-style "update this line" branches) and
#      force-pushes with lease;
#   2. waits until GitHub acknowledges the pushed head SHA — the PR API
#      (headRefOid, statusCheckRollup) lags behind force-pushes by seconds to
#      minutes and will happily report stale data;
#   3. polls the check-runs API for THAT head SHA until all required checks
#      are present and green (note: statuses/conclusions are lowercase);
#   4. merges with the rebase method, retrying transient GraphQL races
#      ("Base branch was modified", "not mergeable"), and verifies the
#      resulting state is MERGED.
# PRs whose required checks fail or that cannot be rebased are skipped, not
# merged. The cargo-audit job is NOT required and fails on pre-existing
# RUSTSEC advisories already present on main — that alone does not skip a PR.
#
# Usage:
#   scripts/merge-renovate-prs.sh            # merge all open renovate PRs
#   scripts/merge-renovate-prs.sh 154 155    # only these PRs
#
# Requires: gh (authenticated), git, jq (bundled with gh is not used; plain
# bash + gh api --jq). The working clone defaults to ./.dev/ocid-merge
# (override with OCID_MERGE_DIR); it is a plain git clone, no containers.

set -uo pipefail
REPO=safonas/ocid
DIR=${OCID_MERGE_DIR:-./.dev/ocid-merge}
REQ_NAMES="fmt + clippy + test|Analyze (rust)|Trivy (image + filesystem)|OpenSSF Scorecard"

if [ $# -gt 0 ]; then
  PRs=("$@")
else
  mapfile -t PRs < <(gh pr list --repo "$REPO" --state open --author app/renovate \
    --json number --jq '.[].number' | sort -n)
fi

mkdir -p "$(dirname "$DIR")"
if [ ! -d "$DIR/.git" ]; then
  git clone --quiet --filter=blob:none "https://github.com/$REPO" "$DIR" || { echo "CLONE FAILED"; exit 1; }
fi
cd "$DIR" || exit 1
git config user.name "$(gh api user --jq .login)"
git config user.email "[EMAIL]"
git fetch --quiet origin main --force --prune
echo "START main=$(git rev-parse origin/main)"
echo "PRs: ${PRs[*]:-none}"

merged=(); skipped=(); failed=()

required_ok() { # $1 = sha; returns 0 when all 4 required check-runs on sha are SUCCESS
  local sha=$1 i res line name status concl
  for i in $(seq 1 60); do # 60*30s = 30 min
    res=$(gh api "repos/$REPO/commits/$sha/check-runs?per_page=100" --paginate --jq \
      '.check_runs[] | "\(.name)|\(.status)|\(.conclusion // "")"' 2>/dev/null || true)
    local pend=0 fail=0 ok=0
    declare -A seen=()
    while IFS= read -r line; do
      [ -z "$line" ] && continue
      name=${line%%|*}; rest=${line#*|}; status=${rest%%|*}; concl=${rest#*|}
      case "|$REQ_NAMES|" in *"|$name|"*) ;; *) continue ;; esac
      status=${status,,}; concl=${concl,,}
      seen["$name"]=1
      if [ "$status" = "completed" ]; then
        if [ "$concl" = "success" ]; then ok=$((ok+1)); else fail=$((fail+1)); fi
      else pend=$((pend+1)); fi
    done <<< "$res"
    if [ "${#seen[@]}" -lt 4 ]; then pend=$((pend+1)); fi # runs not registered yet
    if [ "$fail" -gt 0 ]; then echo "  required check FAILED on $sha"; return 1; fi
    if [ "$pend" = "0" ] && [ "${#seen[@]}" -eq 4 ]; then
      echo "  all 4 required checks green on ${sha:0:7} (cycle $i)"; return 0
    fi
    sleep 30
  done
  echo "  TIMEOUT waiting checks on $sha"; return 1
}

merge_pr() { # retry transient GraphQL races
  local n=$1 a
  for a in 1 2 3 4 5; do
    if gh pr merge "$n" --repo "$REPO" --rebase; then return 0; fi
    echo "  merge attempt $a failed, retrying in 45s"
    sleep 45
  done
  return 1
}

rebase_pr() {
  local n=$1 branch old newsha w cur a
  branch=$(gh pr view "$n" --repo "$REPO" --json headRefName --jq .headRefName) || return 1
  for a in 1 2 3; do
    git fetch --quiet origin "$branch" --force || { sleep 10; continue; }
    old=$(git rev-parse "origin/$branch")
    if git merge-base --is-ancestor origin/main "origin/$branch"; then
      echo "  branch up to date"
      return 0
    fi
    git checkout --quiet -B "rb-$n" "origin/$branch"
    if ! git rebase --quiet origin/main 2>/dev/null; then
      git checkout --theirs -- . 2>/dev/null
      git add -A
      GIT_EDITOR=true git rebase --continue --quiet 2>/dev/null || {
        echo "  REBASE CONFLICT unresolvable"; git rebase --abort 2>/dev/null; return 1; }
    fi
    if git push --force-with-lease="refs/heads/$branch:$old" origin "HEAD:refs/heads/$branch" --quiet 2>/dev/null; then
      newsha=$(git rev-parse HEAD)
      echo "  rebased $branch onto main ($old -> $newsha)"
      for w in $(seq 1 24); do
        cur=$(gh pr view "$n" --repo "$REPO" --json headRefOid --jq .headRefOid 2>/dev/null || echo "")
        if [ "$cur" = "$newsha" ]; then return 0; fi
        sleep 5
      done
      echo "  head SHA did not propagate to GitHub"
      return 1
    fi
    echo "  push raced (stale lease), retry $a"
    sleep 10
  done
  echo "  PUSH FAILED after retries"
  return 1
}

for n in "${PRs[@]}"; do
  echo "=== PR #$n ==="
  state=$(gh pr view "$n" --repo "$REPO" --json state --jq .state 2>/dev/null || echo "MISSING")
  if [ "$state" != "OPEN" ]; then
    echo "  state=$state, skipping"
    [ "$state" = "MERGED" ] && merged+=("$n") || skipped+=("$n:state=$state")
    continue
  fi

  if ! rebase_pr "$n"; then echo "  SKIPPED #$n (rebase)"; skipped+=("$n:rebase"); continue; fi

  head=$(gh pr view "$n" --repo "$REPO" --json headRefOid --jq .headRefOid)
  sleep 15 # let check runs register
  if required_ok "$head"; then
    if merge_pr "$n"; then
      st=$(gh pr view "$n" --repo "$REPO" --json state --jq .state)
      if [ "$st" = "MERGED" ]; then
        echo "  MERGED #$n"
        merged+=("$n")
        git fetch --quiet origin main --force --prune
      else
        echo "  MERGE NOT CONFIRMED (state=$st)"; failed+=("$n:confirm")
      fi
    else
      echo "  MERGE FAILED after retries"; failed+=("$n:merge")
    fi
  else
    echo "  SKIPPED #$n (required checks)"; skipped+=("$n:checks")
  fi
done

echo "===== SUMMARY ====="
echo "merged:  ${merged[*]:-none}"
echo "skipped: ${skipped[*]:-none}"
echo "failed:  ${failed[*]:-none}"
