# GitHub queries for the task scripts, through the REST API only (`gh api`).
# Claude Code cloud sessions can't reach GitHub's GraphQL API, which most
# `gh pr|issue|release|run|repo` subcommands use, so every query goes here.
# `gh api` fills in {owner} and {repo} from the git remote. Source this file;
# each function prints its result and fails when the request fails.

# owner/name of the repository.
gh_repo() { gh api 'repos/{owner}/{repo}' --jq .full_name; }

# The number of the PR merged from branch $1 whose last commit was $2 (a
# squash merge leaves the branch's own commits off main). Empty when none.
gh_merged_pr() {
  gh api "repos/{owner}/{repo}/pulls?state=closed&head={owner}:$1&per_page=100" \
    --jq ".[] | select(.merged_at != null and .head.sha == \"$2\") | .number" | head -1
}

# Open PRs, one per line: number, draft mark, title, link.
gh_open_prs() {
  gh api 'repos/{owner}/{repo}/pulls?state=open&per_page=100' \
    --jq '.[] | "#\(.number)\(if .draft then " (draft)" else "" end) \(.title)  \(.html_url)"'
}

# Tags of draft releases.
gh_draft_releases() {
  gh api 'repos/{owner}/{repo}/releases?per_page=20' --jq '.[] | select(.draft) | .tag_name'
}

# Runs of workflow file $1 that haven't finished: branch or tag, status, link.
gh_unfinished_runs() {
  gh api "repos/{owner}/{repo}/actions/workflows/$1/runs?per_page=20" \
    --jq '.workflow_runs[] | select(.status != "completed") | "\(.head_branch) \(.status) \(.html_url)"'
}

# Open issues (pull requests left out), optionally only those labelled $1:
# number, labels, title.
gh_open_issues() {
  local label=${1:+&labels=$1}
  gh api "repos/{owner}/{repo}/issues?state=open&per_page=50$label" \
    --jq '.[] | select(.pull_request == null) | "#\(.number) \(.title)\(if (.labels | length) > 0 then "  [\([.labels[].name] | join(", "))]" else "" end)"'
}
