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
# The PR merged on GitHub from exactly the last commit of branch $1 (a squash
# merge leaves its commits "unmerged" above).
merged_pr() {
  command -v gh >/dev/null || return 0
  gh pr list --head "$1" --state merged --json number,headRefOid \
    --jq ".[] | select(.headRefOid == \"$(git -C "$root" rev-parse "$1")\") | .number" 2>/dev/null | head -1
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
  gh pr list --state open || echo "!! gh pr list failed"
  echo "== Draft releases (not visible to users until published)"
  gh release list --limit 10 --json tagName,isDraft --jq '.[] | select(.isDraft) | .tagName' || echo "!! gh release list failed"
  echo "== Release builds not finished (queued or running)"
  gh run list --workflow release.yml --limit 10 --json headBranch,status,url \
    --jq '.[] | select(.status != "completed") | "\(.headBranch) \(.status) \(.url)"' || echo "!! gh run list failed"
  echo "== Issues waiting for verification on a real build or platform"
  gh issue list --label needs-verification --state open || echo "!! gh issue list failed"
  echo "== Open issues"
  gh issue list --state open --limit 20 || echo "!! gh issue list failed"
fi
