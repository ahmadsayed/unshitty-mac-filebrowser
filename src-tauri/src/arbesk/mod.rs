//! Arbesk (promptscad.com) accounts in Network, read only: sign in with an
//! email and the code Arbesk emails, then browse the account's collections
//! as folders and their assets as glTF and 3MF files. Arbesk's read path is
//! rebuilt here from public sources (the backend's config and token index,
//! the chain's `tokenURI`, the IPFS gateway); nothing is written to Arbesk.

pub mod chain;
pub mod http;
pub mod ipfs;
pub mod location;
