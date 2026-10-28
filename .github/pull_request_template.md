<!-- Title: conventional and short, e.g. "fix: open files from the Linux AppImage".
     Readable in two minutes. Delete the hints in comments as you fill it in. -->

<!-- One line per issue: "Closes #<n>" if this PR resolves all of it, "Refs #<n>"
     if a part or a verification stays open; "No issue" if there is none. -->
Closes #

## What it's for

<!-- 2–3 sentences, from the user's point of view: what was wrong or missing,
     what they get now. -->

## What changes

<!-- Short bullets by area; settings added with their default. Drop empty ones. -->

- **Interface:**
- **Backend:**
- **Tests:**
- **Docs:**

## Screenshots

<!-- Before / after from `npm run dev:mock` (scripts/mock-screenshot.mjs), light
     and dark when colours change. Upload with scripts/pr-screenshots.sh and paste
     the Markdown it prints. "No visible change" if there is none. -->

| Before | After |
|---|---|
|  |  |

## How it was checked

- [ ] `npx tsc --noEmit`, `npm run build`
- [ ] `npm test` <!-- N passed -->
- [ ] `cd src-tauri && cargo test` <!-- N passed -->
- [ ] CRAP gate (pre-commit hook) passed, no `--no-verify`
- [ ] Looked at in `npm run dev:mock` <!-- or "no UI change" -->
- [ ] Real app / installed build: <!-- what was tried and by whom, or "not needed: …" -->

## Not verified

<!-- What could not be checked here (macOS from Linux, a real launch, real shares…)
     and the needs-verification issue that tracks it. "Nothing" if all was checked. -->

## Follow-up

<!-- What is left out on purpose, and where it is tracked. -->
