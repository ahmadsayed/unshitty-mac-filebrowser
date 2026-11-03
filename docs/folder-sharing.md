# Folder sharing

Share a local folder read-only between File Browser installations on macOS
and Linux. The recipient can browse its contents and open downloaded copies
while your computer is online and File Browser is running.

## Which one?

| | Share Folder… | Send with QR Code… |
|---|---|---|
| For | People with File Browser on macOS or Linux | Any phone or browser, no app |
| What | Read-only access to a whole folder | A copy of one or more files |
| How long | Until you stop sharing (while File Browser runs) | While the dialog is open |
| Who | Anyone with the link, or one person; optional passphrase | Anyone with the code or link |

This guide is about **Share Folder…**. **Send with QR Code…** is on the
right-click menu of files.

## Create a link

1. Right-click a folder and choose **Share Folder…**, or use **Share This Folder…**
   in the path-bar menu. Enter your display name the first time.
2. Optionally select **Protect new links with a passphrase** and enter and
   confirm a strong passphrase. Spaces and capitalization matter.
3. Choose **Copy Link** for anyone with the link, or **Specific Person…** to
   create a link that binds to the first recipient installation that opens it.
4. Send the copied `share://…` link. Send its passphrase separately.

Names are labels, not passwords or verified identities. A Specific Person
link is tied to one installation's cryptographic key, not to the person's
name or email address.

## Open a link

Paste the link into the **Network** address field and choose **Connect**, or
paste it into the location bar (⌘L on macOS, Ctrl+L on Linux).

File Browser recognizes an encrypted invitation automatically and shows
**Enter Passphrase**. Enter the passphrase supplied by the sharer and choose
**Unlock**. An incorrect passphrase or damaged link leaves the dialog open
for retry. **Cancel** adds nothing to Network.

After adding the folder, open it from Network. The passphrase is needed to
unlock the invitation; your installation's identity authorizes later access,
including after restarting the app. An older File Browser version that does
not support protected invitations will ask for a newer version.

## Manage access

The Share dialog lists invitations. Protected ones say **Passphrase protected**.
Use the row's **Copy Link** button to copy the same encrypted invitation again,
even after restarting File Browser; you do not need to re-enter its passphrase.

Passphrase protection applies to **new invitations**. Each protected invitation
gets a new access token; existing links and already-authorized installations
keep their access until removed. To replace a passphrase, create a new
invitation and remove the old row. Remove every unwanted unprotected link if
you intend to distribute only protected invitations. **Stop Sharing…** revokes
the folder's invitations together.

## What is encrypted

- **Browsing and transfers:** iroh uses end-to-end encrypted connections,
  including when traffic passes through its public relays.
- **Protected invitations:** Argon2id derives a key from the passphrase;
  XChaCha20-Poly1305 encrypts and authenticates the complete invitation,
  including the access token, endpoint address, share ID, and sharer name.
  The public envelope contains only a format marker, random salt, nonce,
  and ciphertext with its authentication tag.
- **Local data:** the passphrase and derived key are not saved. The host keeps
  its access tokens and encrypted invitations in private settings files; an
  authorized recipient keeps the unlocked capability in its private
  `received.json`. Local settings and downloaded copies are not encrypted by
  this feature.

Someone holding an encrypted invitation can try passphrase guesses offline,
so choose a strong passphrase. Revocation cannot retrieve downloaded copies,
and anyone who has unlocked an invitation can retain its capability. The
Specific Person rule still rejects claims from other installation identities.

Folder sharing is experimental. Not implemented yet:
- **Live refresh:** the recipient sees the folder as it was when you started
  sharing (or last started File Browser). New files don't appear, removed ones
  still show, and a changed file can't be opened until you restart File
  Browser ([#98](https://github.com/ahmadsayed/unshitty-mac-filebrowser/issues/98)).
- **Copying out:** the recipient can open files but not copy or drag them out
  of the share ([#99](https://github.com/ahmadsayed/unshitty-mac-filebrowser/issues/99)).

## Your network

While anything is shared or received, the app keeps an iroh connection open
on a few UDP ports of its own choosing. It never asks your router to open or
forward a port (no UPnP, NAT-PMP or PCP). When two computers can't reach each
other directly, the traffic goes through one of n0's public relays, still
encrypted, and may be slower.
