#!/usr/bin/env bash
# Upload screenshots for a PR to the `captures` branch (never main), in a
# folder named after the current branch, and print the Markdown to paste into
# the PR. Works before the PR exists. The branch is never checked out: the
# commit is built with plumbing, so no worktree is touched.
#
#   scripts/pr-screenshots.sh before.png after.png
set -euo pipefail

if [[ $# -lt 1 ]]; then
  echo "usage: $0 <image>..." >&2
  exit 2
fi
for file in "$@"; do
  case "$file" in
    *.png | *.jpg | *.jpeg | *.gif | *.webp) ;;
    *) echo "not an image: $file" >&2; exit 1 ;;
  esac
  [[ -f $file ]] || { echo "no such file: $file" >&2; exit 1; }
done

branch=$(git branch --show-current)
[[ -n $branch && $branch != main ]] || { echo "run this on a task branch, not main" >&2; exit 1; }
folder=${branch//\//-}
repo=$(gh repo view --json nameWithOwner -q .nameWithOwner)

parent=""
if git fetch --quiet origin captures 2>/dev/null; then
  parent=$(git rev-parse FETCH_HEAD)
fi

index=$(mktemp)
trap 'rm -f "$index"' EXIT
export GIT_INDEX_FILE=$index
if [[ -n $parent ]]; then git read-tree "$parent"; else git read-tree --empty; fi

markdown=()
for file in "$@"; do
  # Spaces and odd characters would break the Markdown link.
  name=$(basename "$file" | tr -c 'A-Za-z0-9._\n-' '-')
  blob=$(git hash-object -w "$file")
  git update-index --add --cacheinfo "100644,$blob,$folder/$name"
  markdown+=("![${name%.*}](https://github.com/$repo/raw/captures/$folder/$name)")
done
tree=$(git write-tree)
unset GIT_INDEX_FILE

commit=$(git commit-tree "$tree" ${parent:+-p "$parent"} -m "captures: $folder")
# GitHub answers a new branch with "Create a pull request for 'captures'":
# never for this branch, so drop its remote: lines (errors still show).
git push --quiet origin "$commit:refs/heads/captures" 2>&1 | { grep -v '^remote:' >&2 || true; }
printf '%s\n' "${markdown[@]}"
