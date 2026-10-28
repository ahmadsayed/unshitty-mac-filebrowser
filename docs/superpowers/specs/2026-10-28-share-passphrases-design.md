# Optional passphrases for share links

**Date:** 2026-10-28

**Status:** Implemented and verified on macOS.

**Extends:** [Folder Sharing over iroh](2026-10-25-iroh-sharing-design.md), D6 and D9.

## Behavior

Sharing offers **Protect new links with a passphrase**, off by default, for
both Anyone and Specific Person invitations. With it enabled, Copy Link
creates an encrypted invitation using the entered passphrase. The recipient
pastes the ordinary-looking `share://…` link in Network or the location bar;
Files recognizes its public format marker and asks **Enter Passphrase** before
connecting or claiming access. A wrong passphrase or altered ticket leaves
the prompt open for retry. Cancel before unlocking adds nothing to Network.

The passphrase protects the invitation. The existing iroh transport already
encrypts browsing and downloads. After a successful claim, the recipient's
installation identity authorizes future access. A Specific Person invitation
still binds to the first installation that claims it. Display names are labels.

## Link module and format

The Rust link module owns parsing, format detection, encryption and decryption.
The frontend only supplies text and the optional passphrase; it does not
implement cryptography or decode the ticket.

- Keep version 1 plaintext tickets fully compatible, including case-insensitive
  Base32, surrounding whitespace, chat `<…>` wrapping, and one trailing slash.
- Version 2 is `share://` + lowercase, unpadded Base32 of:
  `version (1 byte) | salt (16 bytes) | nonce (24 bytes) | ciphertext + tag`.
- Encrypt the complete version 1 Postcard payload with RustCrypto
  **XChaCha20-Poly1305**. Authenticate a fixed application/format context and
  the entire public header as additional authenticated data.
- Derive the 32-byte key with **Argon2id v1.3**, 64 MiB memory, three iterations,
  one lane. These parameters are fixed by format version, never supplied by an
  untrusted link. Generate a fresh salt and nonce with system randomness.
- Cap decoded tickets at 64 KiB before deserialization; reject malformed or
  truncated headers, oversized and unknown-version inputs before expensive
  work. Ciphertext truncation is detected by authenticated decryption.
- Passphrases are exact UTF-8, case- and whitespace-sensitive, nonempty and at
  most 1,024 bytes. There is no name-based fallback. Show creation confirmation
  fields and send-separately guidance; do not persist the passphrase.
- Run key derivation on a blocking worker with one process-wide permit, held
  by the worker until it finishes. Zeroize Rust passphrases and derived keys.

## Link lifecycle and storage

Each protected invitation gets a fresh token, even if the folder already has
an Anyone link. Store its encrypted ticket with the host's existing access
entry, alongside the token required by Claim. Copying the row returns that
same protected ticket after app restarts without asking for the passphrase.
New optional storage fields default to absent for existing registries.

The single unprotected Anyone link is still reused. Protected Anyone links
and Person links are individually removable. Access rows identify protected
links and provide Copy Link and Remove actions. Creation controls explicitly
apply to **new links**: existing invitations and claimed access keep working
until removed. Changing a passphrase means creating a new invitation and
removing the old one.

The recipient saves the unlocked ticket in its existing private (0600)
`received.json`, as it already does for ordinary invitations. Neither side
stores the passphrase or its derived key. This feature does not encrypt local
settings, downloaded files, or the host's filesystem.

Anyone who obtains a protected link can attempt offline passphrase guesses;
use a strong passphrase and communicate it separately. A recipient who has
unlocked an invitation can retain its capability and downloaded files.

## Interface and UI

- `create_link(share, kind, person?, passphrase?)` returns the invitation and
  an access view with `protected: boolean`.
- `share_link(share, entry)` returns the persisted encrypted ticket for a
  protected entry, otherwise the existing plaintext link.
- `share_link_requires_passphrase(link)` validates the envelope and returns
  its protection status without starting the sharing node.
- `add_received(link, passphrase?)` unlocks locally before any claim or node
  startup. Missing/incorrect passphrases never bind a Person invitation.
- Reuse `openDialog` for unlocking, with a password input, inline error,
  **Cancel** and **Unlock**. Disable duplicate submissions while working.
  Keep cancel disabled during the submitted unlock/claim transaction so it
  cannot claim a share after displaying a successful cancellation.
- Clear passphrase inputs after success or dismissal. They are never included
  in copied text, errors, toasts, localStorage, or settings.

## Verification

- Real Rust crypto: round trips for both kinds; randomized envelopes; exact
  passphrase handling; wrong/missing passphrases; tampered header/ciphertext;
  malformed, truncated, oversized and future-version envelopes; v1 compatibility.
- Real loopback sharing: protected invitation to claim, browse and open;
  no node/claim on failed unlock; copy and restart persistence; fresh tokens;
  second-device refusal for Person; per-link revocation and legacy registry load.
- Frontend against the mock: optional creation for both kinds; confirmation;
  protected row copy; automatic prompt in Network and location bar; incorrect
  passphrase retry, cancellation, own links and ordinary links.
- Required checks: TypeScript, production build, frontend and backend tests,
  CRAP gate, and visual inspection of the mock preview in light and dark modes.
