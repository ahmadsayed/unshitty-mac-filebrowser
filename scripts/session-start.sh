#!/usr/bin/env bash
# Where the project stands, from fresh data: run at the start of a session and
# before answering any question about the project's state (AGENTS.md, "Life
# of a task"). Reports only; it changes and deletes nothing.
set -uo pipefail

root=$(git rev-parse --path-format=absolute --git-common-dir)
root=$(dirname "$root")
git -C "$root" fetch --quiet --prune origin || echo "!! git fetch failed: everything below may be stale"

# Commits of $1 with no identical change on origin/main.
unmerged() { git -C "$root" cherry origin/main "$1" 2>/dev/null | grep -c '^+'; }
# GitHub queries go through REST (scripts/github.sh): cloud sessions have no GraphQL.
source "$(dirname "${BASH_SOURCE[0]}")/github.sh"
# The PR merged on GitHub from exactly the last commit of branch $1 (a squash
# merge leaves its commits "unmerged" above).
merged_pr() {
  command -v gh >/dev/null || return 0
  gh_merged_pr "$1" "$(git -C "$root" rev-parse "$1")" 2>/dev/null
}

echo "== Main checkout ($root)"
read -r ahead behind < <(git -C "$root" rev-list --left-right --count main...origin/main)
echo "main: $ahead ahead, $behind behind origin/main ($(git -C "$root" log -1 --format='%h %s' origin/main))"
dirty=$(git -C "$root" status --porcelain | wc -l)
[[ $dirty -gt 0 ]] && echo "!! $dirty uncommitted change(s) on the main checkout"

echo "== Worktrees"
git -C "$root" worktree list --porcelain | awk '/^worktree /{print $2}' | while read -r dir; do
  [[ $dir == "$root" ]] && continue
  branch=$(git -C "$dir" branch --show-current)
  changes=$(git -C "$dir" status --porcelain | wc -l)
  left=$(unmerged "$branch")
  state="$left commit(s) not on origin/main, $changes uncommitted"
  if [[ $changes -eq 0 ]] && { [[ $left -eq 0 ]] || [[ -n $(merged_pr "$branch") ]]; }; then
    state="merged: finalize with scripts/end-task.sh $(basename "$dir")"
  fi
  echo "${dir#"$root"/} ($branch): $state"
done

echo "== Branches already on origin/main (safe to delete)"
git -C "$root" for-each-ref --format='%(refname:short)' refs/heads refs/remotes/origin | while read -r ref; do
  case $ref in main | origin/main | origin/HEAD | origin) continue ;; esac
  # Skip branches checked out in a worktree: listed above.
  git -C "$root" worktree list | grep -q "\[$ref\]" && continue
  [[ $(unmerged "$ref") -eq 0 ]] && echo "$ref"
done

if command -v gh >/dev/null; then
  echo "== Open PRs (waiting for the human's review and merge)"
  gh_open_prs || echo "!! listing open PRs failed"
  echo "== Draft releases (not visible to users until published)"
  gh_draft_releases || echo "!! listing releases failed"
  echo "== Release builds not finished (queued or running)"
  gh_unfinished_runs release.yml || echo "!! listing release runs failed"
  echo "== Open issues"
  gh_open_issues || echo "!! listing issues failed"
fi
