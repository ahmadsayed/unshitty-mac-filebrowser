//! This computer's files for File Browser, with no Tauri (design
//! `docs/superpowers/specs/2026-11-10-provider-crates-design.md`, step 6):
//! listing, item counts, search, new folders and files, Rename and Batch
//! Rename, path info and the local half of copy and move (`fs_ops`); the
//! renames that never replace an item and the staging names
//! (`rename`); the Trash in both layouts and its log (`trash_ops`); what
//! differs between macOS and Linux for files (`platform`); and this
//! computer's `places::Provider` (`provider`).
//!
//! The app keeps the transfer engine (`engine.rs` and `placement.rs`, D6),
//! Keep Versions as the decorator around this provider (D7), thumbnails, OS
//! icons, Recent and Properties; what this crate needs of them it is handed
//! (the version badge's ownership check, the thumbnailer), so it never
//! depends on the app.

pub mod fs_ops;
pub mod platform;
pub mod provider;
pub mod rename;
pub mod trash_ops;

/// The source guards' walker, shared with the app's guard tests (its
/// `testing` feature), so both scan the same tree.
#[cfg(any(test, feature = "testing"))]
pub mod test_support;

pub use provider::{Local, local};
