#!/usr/bin/env bash
# Finalize a task started with scripts/new-task.sh, once its PR is merged:
# delete its worktree (build output included), its local branch and its
# branch on GitHub, then fast-forward the main checkout if it is clean.
# Refuses while anything would be lost: uncommitted files, or commits that
# are on neither origin/main (directly, rebased or cherry-picked) nor a
# merged PR (a squash merge).
#
#   scripts/end-task.sh appimage-env
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <name>" >&2
  exit 2
fi
name=$1
root=$(git rev-parse --path-format=absolute --git-common-dir)
root=$(dirname "$root")
dir="$root/.worktrees/$name"

if [[ ! -d $dir ]]; then
  echo "no task worktree: $dir" >&2
  exit 1
fi
case "$PWD/" in
  "$dir"/*)
    echo "run this from outside the worktree (e.g. cd $root)" >&2
    exit 1
    ;;
esac

branch=$(git -C "$dir" branch --show-current)
changes=$(git -C "$dir" status --porcelain)
if [[ -n $changes ]]; then
  echo "uncommitted changes in $dir; commit or discard them first:" >&2
  echo "$changes" >&2
  exit 1
fi

git -C "$root" fetch --quiet origin
# "+" lines are commits with no identical change on origin/main. A squash
# merge leaves them all "+", so a PR merged on GitHub from exactly this
# branch's last commit counts as merged too.
unmerged=$(git -C "$root" cherry -v origin/main "$branch" | grep '^+' || true)
if [[ -n $unmerged ]]; then
  head=$(git -C "$root" rev-parse "$branch")
  merged_pr=$(gh pr list --head "$branch" --state merged --json number,headRefOid \
    --jq ".[] | select(.headRefOid == \"$head\") | .number" 2>/dev/null | head -1)
  if [[ -z $merged_pr ]]; then
    echo "$branch has commits that are not on origin/main, and no merged PR of its last commit:" >&2
    echo "$unmerged" >&2
    exit 1
  fi
  echo "PR #$merged_pr merged $branch"
fi

rm -f "$dir/node_modules"
# Only ignored build output (target/, dist/) is left, so --force loses nothing.
git -C "$root" worktree remove --force "$dir"
git -C "$root" branch -D "$branch" >/dev/null
if git -C "$root" ls-remote --exit-code --heads origin "$branch" >/dev/null; then
  git -C "$root" push --quiet origin --delete "$branch"
  echo "Deleted $branch on GitHub"
fi
echo "Removed $dir and branch $branch"

# Bring the main checkout up to date, only when that can't disturb anything:
# it may hold the human's own uncommitted work.
if [[ $(git -C "$root" branch --show-current) == main && -z $(git -C "$root" status --porcelain) ]]; then
  git -C "$root" merge --quiet --ff-only origin/main &&
    echo "main is now at $(git -C "$root" log -1 --format='%h %s')"
else
  echo "Left the main checkout alone (not on main, or it has changes); update it with: git pull --ff-only"
fi
