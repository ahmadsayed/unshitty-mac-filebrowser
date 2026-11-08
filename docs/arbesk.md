# Arbesk in Network

[Arbesk](https://promptscad.com) keeps 3D assets in collections. File Browser
shows an Arbesk account in **Network**: its collections as folders, their
assets as `.gltf` and `.3mf` files, and Space shows a model in 3D.

This is experimental and **read only**: nothing in Arbesk is uploaded,
renamed or deleted from File Browser.

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
(its token index, the chain and its IPFS gateway). The sign-in's session is
ended as soon as Arbesk has confirmed the account, so there is no password
or token to leak and nothing expires. **Disconnect…** forgets the account;
your collections on Arbesk don't change.

Before File Browser keeps an account, Arbesk must confirm it: the session
the browser hands back has to belong to the email you typed and resolve to
the same wallet address. A web page can't slip in another account.

## Browsing

- A collection without a name shows as *Untitled Collection*; an asset
  without a name shows its asset id.
- Assets have no size in the list: an asset's file is only built when you
  open or preview it.
- Opening an asset (double-click, or **Open** in the preview) builds the
  file into the app cache and opens it with your default app. A built file
  is reused while nobody has changed it; files left untouched for a week are
  pruned, and one you edited goes to the Trash instead.
- Copy, Cut and dragging out say *Copying from Arbesk comes next.*; Paste
  into a collection says *Arbesk is read only.*

An asset stored in a way File Browser doesn't know yet, or one that is a
live link to another collection's asset, says *This asset uses an Arbesk
format File Browser can't read yet.* rather than showing a wrong file.

## Another Arbesk

File Browser signs in to `https://promptscad.com` (Base Sepolia). To use
another deployment, a local one for instance, start File Browser with
`FILEBROWSER_ARBESK_ORIGIN`, e.g. `FILEBROWSER_ARBESK_ORIGIN=http://localhost:9090`.
Each account remembers the deployment it signed in to.
