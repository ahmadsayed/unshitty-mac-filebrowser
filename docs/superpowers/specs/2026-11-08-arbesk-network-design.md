# Arbesk in Network — design

Date: 2026-11-08 · Status: proposed, waiting for the human's review

## Goal

Connect an Arbesk account (the 3D asset platform at `https://promptscad.com`,
source in `~/Projects/arbesk`) the way a Google account is connected: sign in
once with an email and the code Arbesk emails, then browse it from Network.

- Network shows one row per connected Arbesk account, labelled "Arbesk" with
  the email under it, like Google Drive.
- Opening it lists the account's **collections as folders**.
- Opening a collection lists its **assets as files**: `<name>.gltf` or
  `<name>.3mf`, in the list and grid views.
- **Space** shows the existing 3D preview of the selected asset; double-click
  opens it with the default app (a copy in the app cache).

Out of scope for this version: every write (New Folder, Rename, Paste or drop
in, Move to Trash, Delete); copying or dragging items out, which SSH servers
and shares don't offer yet either ("Copying from Arbesk comes next."); version history, editor-shared collections from
other chains, search inside Arbesk, thumbnails in the grid. Arbesk's own
repository is not changed.

## Decisions

**D1. Rebuild the read side of Arbesk's protocol in Rust, in this app.**
No bundled `besk` (89 MB, a Bun build step, JavaScript licences to review),
no new Arbesk API. Everything reading needs is public:

| Step | Arbesk source | Rust |
|---|---|---|
| Settings | `GET {origin}/api/v1/config`: contract address, RPC URL and IPFS gateway per chain | one request |
| Collections | `GET {origin}/api/v1/indexer/owned` and `/shared` `?address=&chainId=`: token ids, no session needed | two requests |
| A collection's manifest | `tokenURI(tokenId)` on the chain's RPC, then the manifest's CID on the gateway | hand-written `eth_call` (selector `0xc87b56dd`, ABI-decoded string); no Ethereum crate |
| An asset's manifest and source | `assets[assetID]` → asset manifest CID → `scene.nodes[0].source.cid` when the manifest is a wrapper (asset-core's `resolveCompositeSourceCid`) | requests |
| Stored bytes | brotli frame (asset-core's `utils/brotli-frame.ts`) or gzip, else raw | `brotli` + `flate2` |
| The file | glTF: every `ipfs://<cid>` buffer and image becomes a `data:` URI, `_arbesk` fields stripped (asset-core's `composeGltfJson`). 3MF: `[Content_Types].xml`, `_rels/.rels`, the model and its rels from the composite, binary parts from their CIDs, zipped (asset-core's `3mf/composer.ts`) | `serde_json`, a zip writer |

Origin is `https://promptscad.com`, chain 84532 (Base Sepolia), both stored
with the account so a later local or second deployment is a new account, not
a code change.

**D2. Sign-in is `besk login`'s flow, run by the app.** Online Accounts →
Connect an Account → Arbesk asks for the email in a dialog, then:

1. The backend binds `127.0.0.1:0` and opens
   `{origin}/api/v1/cli-auth?email=<email>&port=<port>` in the default
   browser (the way Drive opens Google's page).
2. Arbesk's page has CDP email the code, verifies it, creates a session and
   redirects to `http://127.0.0.1:<port>/?token=&expiresAt=&address=&email=`.
3. The app answers "You can close this window and return to File Browser.",
   then **checks the callback**: its email must be the one typed, and
   `POST {origin}/api/v1/users/resolve-email` with `Authorization: Session
   <token>` must return that same address. Arbesk's page sends no `state`, so
   this check is what stops another page from handing the app a wrong
   account. A 401 or a different address fails the sign-in; a 502/503 fails
   it with "Arbesk could not confirm this account. Try again later."
4. The app ends the session at once (`DELETE {origin}/api/v1/sessions`) and
   stores only `{ id, email, address, origin, chain_id }` in
   `arbesk_accounts.json` in the app config dir.

The waiting, the deadline, Cancel, ignoring `/favicon.ico` and quiet
connections are the same as Drive's: the loop in `cloud/oauth.rs::finish`
moves into a shared helper both use (Google's callback parsing and token
exchange stay Google's).

**D3. No credential is kept, so access never expires.** Reading needs only
the wallet address, so the 24-hour session is not needed after D2's check.
There is no Sign In Again for Arbesk; `ProviderActions.signIn` opens the
Connect dialog ("Sign In to Arbesk") with the stored email filled in. If Arbesk later requires a session for reads, this
decision is revisited.

**D4. Paths carry ids; names are only for display.** Like Drive's
`<file-id>~<encoded name>` segments:

```
arbesk://<address>                                   the account (Network's row)
arbesk://<address>/<tokenId>~<name>                  a collection
arbesk://<address>/<tokenId>~<name>/<assetID>~<name>.<ext>   an asset
```

`<address>` is the lower-case wallet address, the account key in
`providers.ts`. Names are percent-encoded as Drive's are, so a `/` in a name
can't split a path. A collection without a name is shown as
"Untitled Collection"; an asset without a name uses its asset id. Two items
with the same name both show, since their ids differ. The backend reads only
the ids.

**D5. A new place kind, read-only everywhere.** `paths.ts` gains an `arbesk`
row (`placeOf`, `SCHEMES`, crumbs: the account itself is one crumb, "Arbesk", like "Google Drive"), and
`isRemotePath` includes it. `locations.ts` gains `LocationKind` `"arbesk"`:
read-only, remote, no recursive search, no versions, no new items.
`item-actions.ts` refuses cut, copy, paste, rename, trash, delete, star
and properties there (`copyOutRefusalOf` says "Copying from Arbesk comes
next."; Paste says "Arbesk is read only."); ⌘D may bookmark a collection,
as on an SSH server, and `tests/item-actions-matrix.test.ts`
pins what each surface offers there. `place.rs` gains `Kind::Arbesk`:
`writable` and `transfer` refuse it as a target, `local_only` skips it.

**D6. Listing.** An asset's size is known only once it is built, so its
size column is empty, as a remote folder's item count already is
(`locations.ts` gains `knowsSizes(kind)`, false for `"arbesk"`; the status
bar then says "“name” selected" without a size). The date column shows the
manifest's `timestamp` (milliseconds) when it has one. An asset's extension
comes from its stored source's format, not from the manifest that wraps it;
a live link (`child_ref`) lists as `.gltf` and an unknown format without an
extension (both open with D9's message). Manifests are
content-addressed, so they are cached on disk by CID without expiry (capped
by bytes, oldest dropped first); `tokenURI` and the indexer are asked again
on every listing and on Reload. Requests run at most 8 at a time.

**D7. Open and preview share one built file.** An asset's built
file goes to the app cache (`arbesk/<asset manifest CID>/<name>`; a new
version has a new CID), written under a `.part` name and renamed in with
`placement::rename_new`. `open_target` and `preview_bytes` both ask for it,
so Space then double-click downloads once. It is reused only while
`downloads.rs` says it is unchanged: an edited copy stays where it is and a
fresh one is built beside it, so an edit is never shown as Arbesk's
content. The copy is pruned by `downloads.rs` like other remote downloads
(edited copies go to the Trash). Cached manifests live apart, in
`arbesk-manifests/`, outside the folders pruning walks.

Typed into the path bar, an `arbesk://` location opens as typed; a bare
name typed inside Arbesk, which has no id, asks for "an Arbesk location or
a full path", as Drive does.

**D8. The 3D preview opens remote items of this kind only.** `showPreview`'s
"Preview isn't available for remote items." becomes a `locations.ts` rule,
`previewsRemotely(kind)`, true only for `"arbesk"`. Drive, shares and SSH
keep the message. The built glTF holds only `data:` URIs, so
`gltfCompanions` asks for nothing more. `MODEL_PREVIEW_LIMIT` still applies:
a larger asset shows "This file is too large to preview."

**D9. Unknown formats are refused, not guessed.** A composite whose
`arbesk_format` is neither absent (glTF) nor `composite-3mf`, or a glTF
buffer/image URI that is neither `ipfs://` nor `data:`, fails with
"This asset uses an Arbesk format File Browser can't read yet." An asset
that is a link to another token (`child_ref`) is listed but opens with the
same message.

## Errors

| When | What the user sees |
|---|---|
| The browser sign-in times out (5 min) or is cancelled | Drive's messages: "Sign-in timed out. Try again." / nothing |
| The callback fails D2's check | "Sign-in failed: Arbesk did not confirm this account." |
| promptscad.com, the RPC or the gateway can't be reached while listing | the folder's error page with the reason, as for Drive offline |
| One collection or asset manifest can't be read | that row is left out; the rest list (as asset-core does) |
| Opening an asset whose parts can't all be fetched | the error toast; no partial file stays in the cache (built under a staging name, renamed when complete) |

## Testing

- **Rust, no network:** `eth_call` encoding and string decoding against
  fixed vectors; brotli-frame and gzip detection; glTF compose and 3MF
  compose against fixtures with a fake gateway (`ipfs://` → bytes); the
  sign-in callback check (matching, wrong email, wrong address, 401, 503)
  against a fake backend; path parsing and refusals in `place.rs`'s table
  test.
- **Rust, against Arbesk's own output:** a fixture made once with
  `besk download` from a known CID is checked in with its composite; the
  Rust build of the same composite must be equal (3MF: same entries and
  bytes; glTF: same JSON).
- **Frontend (Vitest + the mock):** the mock gains an Arbesk account with
  two collections, a glTF and a 3MF asset; tests for the provider row,
  Network, the paths and kinds tables, the item-actions matrix, Space on an
  Arbesk asset, and the Connect dialog's email step. Every new command gets
  a case in `tests/mock-core.test.ts`.
- **Real app (the human, with their account):** connect with email + code,
  browse collections, Space on a glTF and a 3MF asset, and open one. macOS's browser hand-off is unverified from Linux; a
  `needs-verification` issue lists it.

## Files

- New `src-tauri/src/arbesk/` (`mod.rs` state and accounts, `chain.rs`
  config/indexer/`eth_call`, `ipfs.rs` gateway reads, decompression and the
  CID cache, `compose.rs` glTF and 3MF, `signin.rs` D2, `location.rs` path
  parsing). Plain functions; only `lib.rs` uses Tauri.
- `src-tauri/src/cloud/oauth.rs`: the loopback wait moves to a shared helper.
- `src-tauri/src/place.rs`, `lib.rs` (the `Kind::Arbesk` arms of
  `list_dir`, `open_target`, `preview_bytes`, `path_info`; commands
  `arbesk_accounts`, `arbesk_connect_start/finish/cancel`,
  `arbesk_disconnect`), `Cargo.toml` (`brotli`, `flate2`, `zip`, all
  already in `Cargo.lock` through other crates, so direct dependencies add no
  new code; still checked with cargo-deny).
- `src/providers.ts`, `src/paths.ts`, `src/locations.ts`,
  `src/item-actions.ts`, `src/api.ts`, `src/main.ts`
  (`PROVIDER_ACTIONS.arbesk`, the email dialog, `showPreview`),
  `dev/mock-tauri/`, the tests above.
- Docs: README (feature list, "Not supported yet": Arbesk writes),
  `docs/architecture.md`, AGENTS.md's layout table.

The uncommitted draft `docs/superpowers/plans/2026-11-08-arbesk-network.md`
in the main checkout (Google-style OAuth against Arbesk) is replaced by the
plan written from this spec.
