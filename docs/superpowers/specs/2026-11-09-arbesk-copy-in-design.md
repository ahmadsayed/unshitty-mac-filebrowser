# Copying files into Arbesk — design

Date: 2026-11-09 · Status: approved in conversation, spec under review

## Problem

Arbesk in Network is read only: its assets can be previewed, opened in the
Studio and (since #240) copied out, but a model on this computer can only
reach Arbesk through the Studio's own upload. The app ends the Arbesk session
right after sign-in and keeps only the wallet address, so it can't act for
the account.

## Goal

Copy and Paste (or a drag) put local `.gltf`, `.glb` and `.3mf` files into an
Arbesk collection as assets, stored the way Arbesk's Studio stores them, so
the Studio, `besk` and this app all read them back.

Out of scope: creating collections, moving, renaming or deleting in Arbesk,
generating thumbnails, sources in Drive, shares or SSH servers, collections
with several editors (D9), and any change to Arbesk itself.

## How Arbesk takes a write (from its code, `~/Projects/arbesk`)

- The sign-in page (`/api/v1/cli-auth`) makes a session (`POST
  /api/v1/sessions`, SIWE) and grants Arbesk's backend a CDP delegation for
  30 days. It redirects to the loopback with `token` and `expiresAt`, which
  the app ignores today.
- Every write call sends `Authorization: Session <token>`. Sessions last
  24 h, have no refresh, and are lost when Arbesk's backend restarts.
- The client does the IPFS work: `POST /api/v1/ipfs/upload-urls {count}`
  returns one-use upload credentials (Pinata `presigned-put`: multipart
  `POST <url>` with `file` and `network=public`, CID in `data.cid`; Kubo
  `kubo-api` in local deployments: `/api/v0/add?cid-version=1` then
  `/api/v0/pin/add`). A Pinata credential expires after 60 s.
- One call changes the chain: `POST /api/v1/wallet/relay`
  `{"op":"updateUri","tokenId":"<n>","params":{"newUri":"<cid>","proof":[],"assetScope":"0x00…00","assetId":"<id>"}}`.
  The backend checks that the session's address owns the collection, then
  sends `updateAssetURI` as the user's smart account, gas paid by the CDP
  paymaster. It answers `{receipt:{transactionHash,status}}` with HTTP 200
  even when `status` is `false` (failed) or `null` (still pending).
- Limits: upload credentials 20/min, relay 30/min (429 with `Retry-After`),
  Studio uploads up to 100 MB.

## Decisions

**D1. What can go in, and where.** Only local regular files named `.gltf`,
`.glb` or `.3mf` (case-insensitive), up to 100 MB each (a `.gltf` with its
parts), into a collection (`arbesk://<address>/<token>~<name>`). Refused, with
one toast before anything starts: folders, other kinds of file and items
from Drive, shares or SSH servers ("Only .gltf, .glb and .3mf files on this
computer can be copied to Arbesk."); the account itself or Network ("Paste
into a collection."); Cut and a forced-move drag ("Arbesk is read only.";
only copies go in). A drop on a collection copies.

**D2. Public data: confirmed on every paste.** Before each paste or drop into
Arbesk, a dialog: heading *Copy to Arbesk?*, text *Files in Arbesk are public:
anyone with their address can download them, and they can't be removed from
IPFS.*, responses Cancel (default) and Copy.

**D3. Names and clashes.** An asset's name is the file name without its
extension, as the Studio stores it. A `.glb` is stored as glTF, so the item
it becomes is `<name>.gltf`: the target a paste checks is the listed name
(`Box.glb` → `Box.gltf`). A taken name asks Replace / Skip / Keep Both as
any copy does. **Replace saves the file as the asset's next version**: same
asset id, `version` + 1, `prev_asset_manifest_cid` = the asset's current
manifest, so the old one stays in its history (‹ › in Space). Keep Both adds
a new asset under the app's usual "(copy)" name.

**D4. The session is kept in the keychain.** Sign-in keeps `token` and
`expiresAt` from the callback in the OS keychain through the same store as
Google Drive's credentials (`KeyringTokenStore`, private-file fallback), under
`arbesk:<account id>`. Nothing secret goes in `arbesk_accounts.json`.
Disconnect… ends the session (`DELETE /api/v1/sessions`) and deletes it.
Reading never needs it. A write with no session, an expired one, or a 401
fails with the provider's sign-in error; the paste then opens Sign In Again
for that account and carries on with the items left once it succeeds (the
Drive pattern). Accounts connected before this version have no session and
sign in once.

**D5. Sign-in security stays as it is.** The callback still has no `state`,
and the check that the typed email's address is the callback's stays. A
session planted by someone else can't write into this account: the relay
refuses a session whose address doesn't own the collection (403, D8).

**D6. How each format is stored: Arbesk's decompose protocol**, as
asset-core does it (`packages/asset-core/src/formats/gltf/glb-parser.ts`,
`gltf/decomposer.ts`, `gltf/dedup.ts`, `3mf/decomposer.ts`), so the Studio,
`besk` and this app read the result exactly as they read the Studio's own
uploads. Proven against asset-core's own output (Testing).
- **Every part** is uploaded as asset-core's `uploadWithDedup` writes it:
  - `_arbesk` = `{hash, hashAlgo:"murmur3-128", compressed, bytes}`, the hash
    MurmurHash3 x86 128-bit (hex, asset-core's `hashBytes`) over the raw
    bytes and `bytes` their length;
  - glTF buffers and images compressed: brotli behind the `ARB\x01` frame
    (`41 52 42 01`), the file name gaining `.br`; 3MF parts stored raw
    (`compressed:false`);
  - identical raw bytes within one file are uploaded once and share a CID
    (the dedup map, scoped to the file being written).
- **`.glb`**: as `decomposeGlb`:
  - each image (by `bufferView` or `data:` URI) becomes a part named
    `{base}_texture_{i}.{ext}`; its entry gets `uri:"ipfs://<cid>"`, its
    `mimeType` (sniffed when missing), `_arbesk`, and loses `bufferView`;
  - the image bytes are cut out of the BIN buffer and every reference is
    re-indexed as asset-core does (bufferViews' offsets, accessors and
    their sparse parts, `KHR_draco_mesh_compression`, the removed views);
  - each buffer becomes a part `{base}_buffer_{i}.bin` with
    `uri:"ipfs://<cid>"`, `byteLength`, `_arbesk`;
  - the composite JSON is written compressed as `{base}_composite.gltf`.
- **`.gltf`**: as asset-core's glTF decomposer for `data:` URIs (same part
  names and fields). A relative URI, which asset-core leaves pointing
  nowhere, is read from the same folder or below (percent decoded; `..`,
  absolute paths and other schemes refused: "“scene.bin” is outside the
  model's folder." / "“scene.bin” can't be found.") and stored the same way.
- **`.3mf`**: a ZIP with `[Content_Types].xml`, `_rels/.rels` and a
  first `*.model` entry, as asset-core picks it, else "This isn't a
  3MF file.". The composite is asset-core's
  `{"arbesk_format":"composite-3mf","modelPath","contentTypes","rootRels","modelRels","model","parts":{"<path>":{"cid","_arbesk"}}}`,
  each other entry a raw part named `{base}_{file name}`.
- **JSON documents** are written as the Studio writes them: the glTF
  composite brotli-framed (`{base}_composite.gltf.br`, the glTF decomposer
  asks for compression), the 3MF composite raw
  (`{base}_composite.3mf.json`), and the asset and collection manifests raw
  (the Studio's `writeJSONToIPFS` compresses only when asked; `besk`
  compresses everything, and every reader takes both).
- `{base}` is the asset name made safe for a file name, as asset-core's
  `sanitizeFileName`. No thumbnail: the asset shows the model icon until
  saved in the Studio.

**D7. The write, per file** (new `src-tauri/src/arbesk/write.rs`, plain
functions over the existing `Http` seam):
1. Check and split the file (D6) before any request.
2. Upload credentials must be Pinata's `presigned-put`; a `kubo-api` one
   (a local Arbesk deployment, whose Kubo is reachable only on its own
   machine) is refused: "This Arbesk server's storage isn't supported."
3. Mint credentials for all parts plus the three JSON documents in one
   `upload-urls` call, upload each part and then the composite; a Pinata
   credential is used at once, so a slow part mints a fresh one when its
   credential is older than 45 s.
4. Upload the asset manifest, the Studio's shape: `type:"asset"`, `name`,
   `asset_id` (`asset_<ms>` new, kept on Replace), `version`, `timestamp`,
   one `source_asset` node (`source: {cid, path:"composite.gltf" |
   "composite.3mf.json", format}`, identity `transform_matrix`,
   `post_processor {color:null, scale 1}`), plus `prev_asset_manifest_cid`
   on Replace.
5. Under a lock per collection (in-process): read `tokenURI` again, add
   `assets[id] = <asset manifest cid>`, `version` + 1,
   `prev_asset_manifest_cid` = the CID read, upload it, call the relay with
   `proof: []`. Holding the lock from the read to the relay's answer keeps
   two pastes from this app from losing each other's asset; a Studio save in
   the same seconds still can (Arbesk has no compare-and-swap).
6. `status: true` is success; `null`: "Arbesk is still saving “Box.gltf”.
   It appears once the change is confirmed."; `false`: "Arbesk couldn't save
   “Box.gltf”." The listing is read again afterwards (manifests are cached by
   CID, so the new collection is fetched fresh).
- Progress counts the bytes uploaded; the relay step shows the operation as
  "Saving to Arbesk…" (it can take up to three minutes: HTTP timeout 4 min).
  Cancel works until the collection manifest is uploaded, never during the
  relay. A cancelled or failed write leaves uploaded parts orphaned on IPFS
  (nothing references them) and the collection unchanged.

**D8. Errors in words.** 401 or no session → the sign-in error (D4); 403 →
"You can't add to this collection."; 429 → "Arbesk is busy. Try again in a
minute."; relay 502 → "Arbesk couldn't save the change. Sign in again to
renew its permission." (an expired CDP delegation looks like this); other
failures say which step: "Couldn't upload “Box.gltf” to IPFS: …".

**D9. Shared collections.** The app sends `proof: []`, which is right when
the account is the only editor. Anything else gets the relay's 403 (D8).

**D10. Backend gates.** `place::copy` gains a route for a local source and
an Arbesk collection target (`CopyRoute::to_arbesk`); `writable` keeps
refusing every other write into Arbesk (move, rename, new folder, trash).
Only `copy_entry` writes into Arbesk.

**D11. The mock** adds an asset to its in-memory collection for a copy into
Arbesk, with the same refusals and the clash rule (D3), and a `needs sign-in`
switch tests can set.

## Testing

- Rust, against `FakeHttp` serving a fake Arbesk (the world `service.rs`'s
  tests already have, plus upload, relay and config endpoints):
  - **protocol conformance**: for each sample asset-core decomposes in
    `scripts/make-arbesk-fixtures.ts` (glb-box, gltf-image, 3mf-box, plus a
    new GLB with an embedded texture), the app's decompose of the same input
    gives asset-core's composite (`source.json`) exactly, CIDs aside, with
    the same `_arbesk` hashes and the same raw part bytes;
  - each format written, then read back with `service::export` and compared
    with the source file;
  - the asset manifest and collection manifest built for a new asset and
    for Replace (`version`, `prev_asset_manifest_cid`, `asset_id` kept);
  - refusals before any request (extension, size, outside or missing glTF
    part, a ZIP without a model);
  - relay `status` true / null / false and 401, 403, 429, 502 mapped to D8;
  - the per-collection lock: two writes in parallel both end up in the
    collection;
  - the session store: saved on sign-in, deleted on Disconnect, a write
    without one gives the sign-in error;
  - `place::copy` table rows for the new route and every refusal.
- TypeScript: the rules (D1), the confirmation dialog (Cancel default),
  `.glb` → `.gltf` target name, Replace / Keep Both, Sign In Again then
  carrying on, the action matrix rows, a drop on a collection, mock-core
  cases.
- Real app, by the human with their account on Base Sepolia (#224): a
  `.glb`, a `.gltf` with a `.bin` and textures, a `.3mf`; each opens in the
  Studio; Replace shows two versions in Space; Keep Both; an expired session.
