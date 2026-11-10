//! One interface for every place File Browser shows (this computer, Google
//! Drive, Arbesk, SSH servers, received shares): the types every place
//! returns and the helpers they share, so no provider reaches into another
//! one or into the app for them.
//!
//! The interface, wired into the app's read side (listing, existence, path
//! info, thumbnails, opening) since step 2, Arbesk's commands and copies
//! since step 3, and Google Drive's copies and moves since step 4: a
//! [`Provider`]
//! per kind of place (blocking, POSIX-like verbs on path strings), the
//! [`Transfer`] primitives the app's transfer engine builds copy and move
//! from, the optional [`ext`] traits, typed [`Error`]s worded as today, and
//! the [`Registry`] that picks a provider by its path's `<scheme>://`.
//!
//! See `docs/superpowers/specs/2026-11-10-provider-crates-design.md`. This
//! crate depends on nothing but `serde` and `serde_json`; no crate but the app depends on
//! Tauri.

mod entry;
mod error;
mod files;
mod info;
mod progress;
mod provider;
mod records;
mod refusal;
mod registry;

pub use entry::{CloudEntry, Entry, OpenTarget};
pub use error::{Error, Result};
pub use files::{MARK_SUFFIX, is_unchanged, mark, mark_path, now, unchanged, write_json};
pub use info::{FileProperties, Measure, PathInfo};
pub use progress::{
    CANCELLED, Phase, Progress, SLICE, Snapshot, cancelled_io, copy_counted, is_cancel,
};
pub use provider::{
    Capabilities, Cx, Existing, Extensions, Fetch, Fetched, Landing, ListOptions, NewKind, Operation,
    Provider, Put, Reading, Refused, Replace, Source, Transfer, Trashed, Unread, ext,
};
pub use records::{Json, Shared, SignIn, Version};
pub use refusal::Refusal;
pub use registry::Registry;
