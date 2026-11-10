# Handover: provider refactor, step 6b onwards

Written 2026-11-10 by a Claude Code cloud session, to be continued in Claude
Code on the desktop. **Delete this `handover/` folder before opening the PR
for this branch.**

## What the owner asked for

"Go full automated mode, do the PR to Arbesk, merge it, as well as the
required changes to the file browser, after a clean refactor; take all the
decisions and merge the PRs."

So:

1. **The provider refactor.** Every connector is a crate behind one POSIX-like,
   path-addressed interface (`places`).
2. **Then representations support in the file browser.**

The agent opens, reviews and merges each PR itself. AGENTS.md's "the human
merges" rule is waived for this work. The owner also said: "skip the commit
hook on incremental commits, run it once at PR time". So work-in-progress
commits use `--no-verify`, and the CRAP gate runs once on the finished branch,
before the PR:

- `node scripts/crap-gate.mjs --rust` for a Rust-only branch;
- `node scripts/crap-gate.mjs` when frontend files changed.

Everything else in AGENTS.md still holds:

- never lose data;
- never change `.crap-baseline.json` except on purpose;
- never `npm ci` in a worktree;
- never touch the updater signing key or approve the `release` environment.

## Read first

- Design: `docs/superpowers/specs/2026-11-10-provider-crates-design.md`
  (D1–D11, Migration steps 1–8 and 6b, Risks, Testing).
- `AGENTS.md` (layout table, data-safety rules, life of a task).
- `docs/architecture.md`.

## Where things stand (`main` at 9c6d2f0)

| Step | Status | PR |
|---|---|---|
| 1 `places` crate: shared types and interface | merged | #253 |
| 2 Registry and read-side dispatch | merged | #254 |
| 3 Arbesk implements `Provider` in its crate | merged | #255 |
| 4 Engine (`engine.rs`) runs Drive transfers; `places-gdrive` crate | merged | #256 |
| 5 `places-sftp` crate; received shares as a provider; no scheme checks left | merged | #257 |
| 6 `places-local` crate (`fs_ops`, `trash_ops`, `platform`, `rename`, `Local`) | merged | #258 |
| **6b Local writes and local `Transfer` through the registry, with the Keep Versions decorator** | **work in progress on this branch** | — |
| 7 Capabilities reach the frontend | to do | — |
| 8 Docs pass | to do | — |
| Representations in the file browser | to do | — |

Arbesk side, done: representations are stored and carried through every
save, and `besk` gains attach, detach, info and `download --rep`
(ahmadsayed/arbesk#139, merged). Its E2E test was never run (ahmadsayed/arbesk#140:
it needs Docker and the Solidity compiler).

At step 6, `cargo test --locked` passed 1,164 tests: app 586, `places-local` 120,
`places-gdrive` 140, `places-sftp` 11, Arbesk 265, `places` 39, `release_signing` 3.

## Step 6b: this branch (`refactor/places-local-writes`)

The spec's step 6b text says what it is. In short:

- Keep Versions becomes a decorator in the app (`src-tauri/src/versions/versioned.rs`,
  new) wrapping `places_local::Local`, and is what the registry holds for
  this computer.
- The local provider's `create`, `rename`, `trash` and `restore` act.
- Its `Transfer` (`crates/places-local/src/transfer.rs`, new) gives the engine:
  - `rename_new` as the atomic rename;
  - `Trash::send` / `Trash::untrash` as Replace's send and take-back;
  - the staged write (`Stage::by_kind`).
- Commands then go through the registry: New Folder and New File, Rename,
  Batch Rename, Move to Trash, Put Back, and local copy and move.
- `engine.rs` runs local↔local routes from the primitives, as it runs Drive's.
- `placement.rs` keeps only what the engine doesn't (downloads' `Target::place`).
- `local_transfer.rs` is deleted (absorbed).

**State of the work in progress:** the implementer was stopped mid-way. Its
uncommitted work is committed here as one WIP commit (`--no-verify`), about
1,500 lines across 12 files. Nothing in it has been reviewed, and the tests
have not been run.

- `cargo check --tests` passes for the app, `places` and `places-local`.
- The `arbesk` crate's tests do **not** compile: `crates/arbesk/src/provider.rs:478`
  still builds `Trashed { result: read_only.clone() }` as `Result<String>`,
  while `places::Trashed.result` is now `Result<Option<String>>`. Update that
  test (and check any other `Trashed` users).
- First check whether the implementation is complete against the spec's 6b
  text (`git diff origin/main...HEAD`). Then finish it.

**This is a data-safety step.** Every "Never lose data" rule in AGENTS.md must
hold exactly:

- **Staging:** a new file is staged and appears whole; a new folder fills in
  place.
- **Replace:** stage the new item, trash the old one through the logged
  `Trash::send`, rename the new one in, and take the old one back unless
  something took its place.
- **A move that replaces nothing** is one `rename_new` onto the target.
- **A cross-volume move** deletes the source only after the copy fully
  succeeded.
- **Refusals** keep `placement::Refusal`'s order and wording.
- **Keep Versions** captures happen exactly as before.
- **The Trash** works for both layouts.

Moved sequences keep their test bodies unchanged. Time copying a
10,000-file folder before and after.

**To finish 6b:**

1. Fix the compile error. Review the diff against the spec, and finish what
   is missing.
2. Run `cargo test --locked`, `npx tsc --noEmit` and `npm run build`.
3. Get two independent reviews, read-only: one on behaviour and data safety,
   one on design fit. Then fix the findings and recheck the fix commits. In
   every step so far, the recheck of the fixes found a real bug.
4. Run the CRAP gate once (`node scripts/crap-gate.mjs --rust`). It runs every
   test with coverage. It keys scores by file path, so moved functions can
   count as new: split or test them, and don't move baseline entries except
   identical macOS-only scores, said in the commit message.
5. Delete `handover/`, push, and open the PR following the template (see
   #258's body for the shape). Post a self-review comment, then squash-merge
   with `gh api -X PUT repos/{owner}/{repo}/pulls/<n>/merge -f merge_method=squash`.
6. Finalize with `scripts/end-task.sh places-local-writes`.

`handover/provider-step.js` is the Workflow script the cloud session used for
each step. Its phases are implement, two reviewers, fix, recheck of the fixes,
and the final gate. Its args are `{step, worktree, branch, task}`. Its paths
point at the cloud container (`/home/user/unshitty-mac-filebrowser`): change
`MAIN` before using it.

## Step 7: capabilities reach the frontend (D5)

- **The command:** a command returns each provider kind's `Capabilities`
  (booleans only to the frontend).
- **Mock and tests:** a mock in `dev/mock-tauri/`, a case in
  `tests/mock-core.test.ts`, and a shared fixture so Rust and TS agree.
- **The frontend:** reads it once at startup into a cache.
- **What it replaces:** `src/locations.ts` and `src/item-actions.ts` stop
  restating what each place allows wherever the table can answer.
  `tests/locations.test.ts`, `tests/item-actions.test.ts` and
  `tests/item-actions-matrix.test.ts` must keep passing (what every surface
  offers must not change).
- **Arbesk:** its capabilities depend on the path (only an account creates;
  copy-out only from an asset; copy-in only into a collection), so it needs a
  row each for account, collection and asset, or a per-path rule.
- **Checks:** this is the first frontend step. Run `npm test`, and the full
  gate (both halves). Take screenshots per AGENTS.md if anything visible
  changes (nothing should).

## Step 8: docs

- **AGENTS.md:** the layout table, gotchas and the "Rules the code must keep"
  section, so they describe the crates as built.
- **Architecture:** `docs/architecture.md` and `docs/architecture.svg`.
- **Spec:** status "done" with a "Where things stand" section, and the README
  if anything user-facing is described.
- **Verification issue:** open a `needs-verification` issue covering the
  refactor. Each line should say what to try and what to expect:
  - **Real app on Linux:** Drive (copy, move and Replace both ways), SSH
    listing and open, a received share, Arbesk (sign-in, list, preview,
    share, copy in and out), and local copy, move, Replace, Trash and
    Put Back.
  - **macOS:** `cargo test` on a Mac (step 6 moved macOS-only code
    byte-identical: `renamex_np`, Finder's Trash and `.DS_Store` Put Back,
    the macOS copy path), plus the same real-app checks.

## Then: representations in the file browser (task 29)

Arbesk now stores `representations` on asset manifests:

- **Fields:** `id`, `role` (source, mesh, view, machine, other), `format`,
  `name`, `cid`, `bytes`, `encoding` (raw or brotli), `derived_from {ref, cid,
  transform_matrix?}`, `tool`, `profile`, `stats` and `created_at`.
- **Staleness:** computed, never stored (`isStale`).
- **Format spec:** in the Arbesk repo,
  `docs/superpowers/specs/2026-11-10-asset-representations-design.md`
  (R1–R8).

The owner's idea: the user sees one file. Print sends the G-code. Open With a
slicer or Blender lets the user pick the 3MF, G-code, STL or `.blend`. The
owner preferred "Arbesk native".

Planned scope, in this order:

1. **Read representations in the `arbesk` crate.** Parse the manifest field,
   compute staleness the same way, and pass them into `Entry` and Properties.
2. **Badges and a stale dot** on assets that carry other files.
3. **Open With:** pick a representation when the chosen app takes one of its
   formats.
4. **Copy out:** choose which file to copy (the model or a representation).
5. **Attach:** paste or drop a file onto an asset to attach it as a new
   version, the same write `besk attach` does (R4: version + 1, `prev` link,
   compare-and-swap on the collection entry).
6. **Space preview and Properties:** a switcher in the preview, and a "Files"
   tab in Properties.
7. **Later:** Print, and a G-code viewer.

Write a design spec first in `docs/superpowers/specs/`, as the refactor did.

## Housekeeping owed

- **Old branches:** the cloud proxy could not delete remote branches.
  Delete them:
  `refactor/places-crate`, `refactor/places-registry`, `refactor/places-arbesk`,
  `refactor/places-gdrive`, `refactor/places-sftp` and `refactor/places-local`
  (all merged).
- **Open Dependabot PRs:** #244 (minor and patch), #245 (md-5 0.11) and
  #246 (iroh-blobs 0.104). Review them like any PR. #121 (bao-tree) and #122
  (gio) broke builds before.
- **Tags:** tags pushed from the cloud went to the wrong commit twice. Tag an
  explicit SHA.
- **Not verified, earlier this session:** #128 (Finder's full folder icon on
  macOS 26) and #105 (signed updates). If this desktop is a Mac, those and the
  macOS points above can be checked here.

## Session start on the desktop

1. Run `scripts/session-start.sh`.
2. `git fetch origin refactor/places-local-writes`, then
   `git worktree add .worktrees/places-local-writes refactor/places-local-writes`.
   Link `node_modules` the way `scripts/new-task.sh` does.
3. Run `npm run build` before `cargo` (`generate_context!` reads `../dist`).
