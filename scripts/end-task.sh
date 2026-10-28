#!/usr/bin/env bash
# Finish a task started with scripts/new-task.sh: once its work is on
# origin/main, delete its worktree (build output included), its local branch
# and its branch on GitHub, if pushed. Refuses while anything would be lost:
# uncommitted files, or commits that are not on origin/main, directly or as
# an identical change (rebased or cherry-picked).
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
# "+" lines are commits with no identical change on origin/main.
unmerged=$(git -C "$root" cherry -v origin/main "$branch" | grep '^+' || true)
if [[ -n $unmerged ]]; then
  echo "$branch has commits that are not on origin/main; merge them first:" >&2
  echo "$unmerged" >&2
  exit 1
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
