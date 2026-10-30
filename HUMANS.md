# HUMANS.md — your side of the workflow

Agents (Claude Code and others) follow [AGENTS.md](AGENTS.md). This page is
the other half: what you do, and what you can expect from them.

## Who does what

| The agent | You |
|---|---|
| Works in its own worktree, `.worktrees/<task>`, never in your checkout | Keep your own work in the main checkout; agents don't touch it |
| Tests, takes screenshots, opens a PR from the template | Choose tasks |
| Reviews its own PR and posts a verdict: ✅ ⚠️ ❌ | **Review and merge** the PR, or ask for changes |
| Asks you to try what it can't (a real build, a Mac, a click it can't make) | Try it and say what happened |
| Finalizes after your merge: worktree and branches deleted, your `main` updated | Say "finalize" if a session ended before it did |
| Prepares releases and tests the draft | Ask for a release; say when to publish |

**Nothing reaches `main` unless you merged it.** Agents push only their task
branches, and merge only when you tell them to ("merge"). GitHub enforces the
first part: `main` is protected, so a direct push is refused for everyone,
you included, as are force pushes and deleting `main`. A PR needs no
approval (GitHub won't let you approve your own), and tags, so releases, are
unaffected. Agents act through your `gh` account, so GitHub can't stop one
from merging a PR: that part stays a rule in AGENTS.md. To change the
protection: Settings › Branches on GitHub.

## Life of a task

1. You ask for something ("fix opening files from the AppImage").
2. The agent creates `.worktrees/<name>` on branch `<kind>/<name>`, from the
   latest `main`.
3. It writes a test, makes the change, runs every check, and looks at the
   result in the mock preview or the real app.
4. It opens a PR: what it's for, what changes, before/after screenshots, how
   it was checked, and what isn't verified.
5. It posts a review comment with a verdict:
   - ✅ ready to merge;
   - ⚠️ something for you to decide, with options;
   - ❌ blocked, and why.
6. **You merge** (any merge button works: merge, squash or rebase), or
   comment with what to change; the agent pushes fixes to the same PR.
7. The agent finalizes: `scripts/end-task.sh <name>` deletes the worktree and
   the branch (local and on GitHub), and fast-forwards your `main` if it has
   no uncommitted changes.

## Useful commands

```sh
scripts/session-start.sh        # where things stand: main vs GitHub, open PRs, worktrees, drafts, issues
scripts/end-task.sh <name>      # finalize a merged task yourself
gh pr list                      # PRs waiting for you
gh pr checkout <n>              # try a PR's branch locally
```

## Things only you can check

- **The other platform.** Work done on Linux can't be checked on macOS from
  here, and the reverse. Open issues labelled `needs-verification` list what
  is left, one checklist line per point (for example #30): tick them off when
  you're on that machine.
- **A real build.** When a change touches packaging or launching other
  programs, the agent asks you to try the installed app, saying exactly what
  to do and what you should see.

- **Phones.** Share with QR Code… downloads on a phone through a page on
  GitHub Pages. Turn Pages on once (Settings → Pages → Source: GitHub
  Actions); the File page workflow publishes it from `main`, or by hand.
  Before that, `web/file-get/serve.sh` serves the page from your computer
  to a phone on the same Wi-Fi, and prints how to start the app.

## Releases

1. Ask: "release 0.5.2". The agent opens a `chore: release 0.5.2` PR; you
   merge it.
2. It tags the merge, and the Release workflow builds macOS and Linux into a
   **draft** release that nobody else can see.
3. It downloads the draft, checks it, installs it here, and asks you to try
   it. On the other platform, that's yours to do or note on an issue.
4. You say "publish" (or run `gh release edit v0.5.2 --draft=false`).

A draft can still be fixed: delete it and move the tag. Once published, a
version is permanent; a later fix ships as the next patch version.

### The updater signing key

Check for Updates installs only packages signed with your key. You made it
once:

```sh
npm run tauri signer generate -- -w ~/.tauri/filebrowser.key
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/filebrowser.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

Keep `~/.tauri/filebrowser.key` and its password backed up (a password
manager or an encrypted drive). If either is lost, installed copies can't
update any more: everyone has to reinstall by hand once a new key is in
`tauri.conf.json`. Agents never touch the private key. Without the secrets a
tag push fails at "Write latest.json" instead of drafting a release that
installed copies couldn't update from; a manual or pull-request run still
builds, just without `latest.json`.

## Where things are

- `.worktrees/` holds the agents' task folders (ignored by git).
- The `captures` branch holds PR screenshots; it never merges into `main`.
- `web/file-get/` is the phone page for QR shares (GitHub Pages).
- `docs/linux-appimage.md` and `scripts/install-linux-appimage.sh` install
  the AppImage for GNOME.
