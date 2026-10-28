#!/usr/bin/env bash
# Start a task in a worktree of its own, from an up-to-date origin/main:
# .worktrees/<name> on branch <kind>/<name>. The main checkout, and any
# uncommitted work in it, is left alone. See "Life of a task" in AGENTS.md.
#
#   scripts/new-task.sh fix appimage-env
set -euo pipefail

kinds="feat fix perf refactor docs chore"
if [[ $# -ne 2 || " $kinds " != *" $1 "* || ! $2 =~ ^[a-z0-9][a-z0-9.-]*$ ]]; then
  echo "usage: $0 <$(tr ' ' '|' <<<"$kinds")> <name: lowercase letters, digits, dots and dashes>" >&2
  exit 2
fi
kind=$1
name=$2
branch="$kind/$name"

root=$(git rev-parse --path-format=absolute --git-common-dir)
root=$(dirname "$root")
dir="$root/.worktrees/$name"

if [[ -e $dir ]]; then
  echo "already exists: $dir" >&2
  exit 1
fi
if git -C "$root" show-ref --verify --quiet "refs/heads/$branch"; then
  echo "branch $branch already exists; finish or remove it first (scripts/end-task.sh $name)" >&2
  exit 1
fi

git -C "$root" fetch --quiet origin
git -C "$root" worktree add --quiet -b "$branch" "$dir" origin/main
# Share the main checkout's packages instead of installing them again.
if [[ -d $root/node_modules ]]; then
  ln -s "$root/node_modules" "$dir/node_modules"
else
  echo "note: no node_modules in $root; run npm install there first" >&2
fi

echo "Task worktree: $dir"
echo "Branch:        $branch (from origin/main at $(git -C "$dir" rev-parse --short HEAD))"
