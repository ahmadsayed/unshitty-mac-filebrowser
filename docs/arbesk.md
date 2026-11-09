# Arbesk in Network

[Arbesk](https://promptscad.com) keeps 3D assets in collections. File Browser
shows an Arbesk account in **Network**: its collections as folders, their
assets as `.gltf` and `.3mf` files, and Space shows a model in 3D. You can
copy `.gltf`, `.glb` and `.3mf` files from this computer into a collection
(see [Copying into Arbesk](#copying-into-arbesk)).

This is experimental. Nothing in Arbesk is renamed or deleted from File
Browser, and nothing is added except by that copy.

## Connecting

1. Open **Online Accounts…** (main menu, or Network's menu) and choose
   **Arbesk** under *Connect an Account*.
2. Type the email you use with Arbesk and choose **Open Browser**.
3. Arbesk's page opens in your browser and Arbesk emails you a code. Type it
   there. The page then hands back to File Browser, which says
   *Connected to …* and opens your collections.

This is the same sign-in as `besk login`. File Browser waits on a port of
this computer (`127.0.0.1`) for the browser to come back, for at most five
minutes; **Cancel** stops waiting.

## What is kept

Only your email, your wallet address and which Arbesk it is
(`arbesk_accounts.json`, beside the app's other settings). Browsing needs
nothing more: collections and assets are read from Arbesk's public sources
(its token index, the chain and its IPFS gateway). The account file holds no
credential. The session from the sign-in is kept apart, in the system
keychain (a file only you can read in the app's settings folder when there
is no keychain), for 24 hours, and used only to copy files in. **Disconnect…**
ends that session and forgets the account; your collections on Arbesk don't
change. Accounts connected before copying existed sign in once more when you
first copy.

Before File Browser keeps an account, Arbesk must confirm it: with the
session the browser hands back, File Browser asks Arbesk which wallet
address the email you typed has, and keeps the account only if Arbesk
accepts the session and names the same address. So a web page can't slip
in someone else's account. This shows the address is your email's; it
doesn't show the session came from this sign-in, which is enough because
browsing reads only public data and needs no session.

Arbesk's sign-in page is Arbesk's own, the one `besk login` opens; besides
signing you in it sets up what Arbesk needs to make changes for you.

## Browsing

- A collection without a name shows as *Untitled Collection*; an asset
  without a name shows its asset id.
- Assets have no size in the list: an asset's file is only built when you
  open or preview it.
- An asset saved with a thumbnail in Arbesk shows it in the grid and list
  views (with **Show Thumbnails** on); one saved without keeps the model's
  icon.
- Opening an asset (double-click, **Open** in its menu or in the preview)
  opens it in Arbesk's Studio, in your browser, on the deployment you signed
  in to (`/studio?asset=<collection>&assetId=<asset>`, the link `besk show`
  opens). Space builds the asset into the app cache for the 3D preview: a
  glTF asset as one binary `.glb`, so the preview has no base64 to decode
  (the asset itself stays a `.gltf`); a built file is reused while nobody has
  changed it, and files left untouched for a week are pruned.
- There is no Copy Location: an `arbesk://` address means nothing outside
  File Browser.
- **Copy** an asset, then **Paste** it into a folder or Google Drive (or
  drag it there): File Browser builds the asset's file as Arbesk does (a
  `.gltf` with every part inside it, or the `.3mf`) and copies it under its
  listed name. It's the newest version, the one the listing shows. A name
  already taken asks *Replace*, *Skip* or *Keep Both*, as for any copy. The
  built file waits in the app cache beside the preview's and is pruned the
  same way; the copy you pasted is yours and is never pruned.
- Cut, and a drag that forces a move, say *Arbesk is read only.* or do
  nothing. Pasting into the account or Network says *Paste into a
  collection.*
- A collection can't be copied yet (*Copying Arbesk collections comes
  next.*), nor can a scene that links to other assets (*This asset uses an
  Arbesk format File Browser can't read yet.*). Assets can't be dragged to
  other apps: they stay in File Browser, as Google Drive items do.

An asset whose parts add up to more than 512 MB says *This asset is too
large to open.*, and a collection that can't be reached (Arbesk, its chain
or its storage offline) shows the reason instead of an empty folder.

An asset can be a scene: several models, each placed where the Studio put
it, some of them links to other assets. Space shows the whole scene: each
model is downloaded once however many times it appears, links are followed
(up to 5 deep, never in a loop), and a model's colour and scale from the
Studio are applied. A link to an asset on another chain is left out, as is
any model that can't be built; the rest of the scene still shows. A scene
stops at 2,000 copies. Each model is shown the right way round; the Studio
currently shows them mirrored (ahmadsayed/arbesk#138).

An asset saved more than once in the Studio has versions: each save points
at the one before it. Space shows the newest, and ‹ and › in the preview's
header step back through the saves (the newest 50, as the Studio reads
them), each shown as its own scene. A model that didn't change between
saves is downloaded once. Older saves store the model file itself (a `.glb`, or
a zipped `.3mf`) instead of a stored source; the preview shows those as
they are, as the Studio does.

A model stored in a way File Browser doesn't know yet says *This asset uses
an Arbesk format File Browser can't read yet.* rather than showing a wrong
file.

## Copying into Arbesk

Copy `.gltf`, `.glb` or `.3mf` files in File Browser (or drag them from
another folder) and paste into a collection. Only those files on this
computer, up to 100 MB each (a `.gltf` counts its own files, a 3MF its
unzipped contents), are accepted; folders, other files and items from other
places say *Only .gltf, .glb and .3mf files on this computer can be copied
to Arbesk.* Nothing is read outside the file's own folder.

**Files in Arbesk are public.** Every paste first asks *Copy to Arbesk?*:
anyone with a file's address can download it, and it can't be removed from
IPFS. Cancel is the default.

How it is stored: File Browser does what Arbesk's Studio does. A `.glb`'s
textures become separate parts, a glTF's parts are compressed, a 3MF's are
stored as they are, and each file becomes an asset with a manifest in the
collection. A `.glb` is listed as a `.gltf`. The asset has no thumbnail
until it is saved in the Studio.

A name already taken asks *Replace*, *Skip* or *Keep Both*. Replace saves
the file as the asset's newest version; the earlier ones stay in its
history (‹ and › in Space), and the dialog says so. Keep Both adds
*Name (copy)*.

The progress shows the bytes uploaded, then *Saving to Arbesk…* while
Arbesk saves the change on the chain, which can take a few minutes.
**Cancel** works until the collection is about to change.

With no session, or one older than 24 hours, **Sign In Again** opens and the
paste carries on after it (once per paste). Reading never needs a session.

Two copies from File Browser into one collection wait for each other. If an
earlier save may still be landing (unconfirmed, timed out, or a server
error), a later one waits up to 30 seconds and then says *Arbesk is still
saving an earlier copy into this collection. Try again in a minute.* Arbesk
has no way to detect it, so a save from the Studio in the same few seconds
can still overwrite the change.

Other messages: *You can't add to this collection.* (not yours to change),
*Arbesk is busy. Try again in a minute.*, *Arbesk couldn't save the change.
Sign in again to renew its permission.*, *Arbesk is still saving “name”. It
appears once the change is confirmed.*, *Arbesk couldn't save “name”.*,
*This isn't a 3MF file.*, *This isn't a glTF file.*, *“uri” is outside the
model's folder.*, *“uri” can't be found.*, *This file is too large for
Arbesk (100 MB at most).*

Only Arbesk's own storage (Pinata) is supported. A local Arbesk that uses
its own IPFS node says *This Arbesk server's storage isn't supported.*

## Another Arbesk

File Browser signs in to `https://promptscad.com` (Base Sepolia). To use
another deployment, a local one for instance, start File Browser with
`FILEBROWSER_ARBESK_ORIGIN`, e.g. `FILEBROWSER_ARBESK_ORIGIN=http://localhost:9090`.
Each account remembers the deployment it signed in to.
