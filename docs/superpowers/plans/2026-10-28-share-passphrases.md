# Optional share passphrases — implementation plan

**Spec:** [Optional passphrases for share links](../specs/2026-10-28-share-passphrases-design.md)

- [x] Inspect current link, authorization, persistence, dialog and mock flows;
  record the approved behavior and encrypted envelope design.
- [x] Implement versioned encrypted tickets with Argon2id and
  XChaCha20-Poly1305, bounded parsing and blocking-worker key derivation.
- [x] Add protected invitation persistence, fresh token creation, protected
  copy, passphrase-aware receive, and Tauri/TypeScript interfaces.
- [x] Add creation fields and protected access-row actions; automatic unlock
  dialogs for Network and the location bar; extend the mock.
- [x] Verify real cryptography and loopback access rules, registry compatibility,
  frontend creation/unlock/retry/cancel flows and every new mock command.
- [x] Update user documentation and inspect the light/dark mock preview.
- [x] Run `npx tsc --noEmit`, `npm run build`, `npm test`, backend `cargo test`
  and the CRAP gate; record results and any environment limitations.

## Verification notes

- `npx tsc --noEmit` and `npm run build`: passed.
- `npm test`: 210 tests passed.
- `cargo test`: 538 tests passed on macOS.
- The crypto compatibility fixture was generated independently with
  argon2-cffi and libsodium/PyNaCl, using the documented version 2 parameters.
- Playwright against `npm run dev:mock`: protected creation/copy, automatic
  unlock, wrong-passphrase retry and successful receive passed. Light/dark
  screenshots were inspected; no page errors occurred.
- Corrected two pre-existing macOS test assumptions: non-UTF-8 filenames are
  a Linux-only fixture, and the sharing Trash predicate uses a canonical path.
- The first CRAP run exposed an existing macOS thumbnail coverage gap:
  Quick Look bypassed raster decoding. Its concurrency test now exercises the
  decoder directly and checks each output's dimensions. `npm run crap` now
  passes for both Rust and TypeScript.
- Linux execution and real-relay connectivity were not exercised here;
  sharing integration tests use real iroh connections over loopback.
