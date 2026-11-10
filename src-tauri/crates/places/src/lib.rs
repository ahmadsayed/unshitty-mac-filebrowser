//! One interface for every place File Browser shows (this computer, Google
//! Drive, Arbesk, SSH servers, received shares): the types every place
//! returns and the helpers they share, so no provider reaches into another
//! one or into the app for them.
//!
//! The interface itself is declared here but not wired yet: a [`Provider`]
//! per kind of place (blocking, POSIX-like verbs on path strings), the
//! [`Transfer`] primitives the app's transfer engine builds copy and move
//! from, the optional [`ext`] traits, typed [`Error`]s worded as today, and
//! the [`Registry`] that picks a provider by its path's `<scheme>://`.
//!
//! See `docs/superpowers/specs/2026-11-10-provider-crates-design.md`. This
//! crate depends on nothing but `serde`; no crate but the app depends on
//! Tauri.

mod entry;
mod error;
mod files;
mod info;
mod progress;
mod provider;
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
    Capabilities, Cx, Extensions, Fetch, Fetched, ListOptions, NewKind, Operation, Provider, Put,
    Refused, Replace, Transfer, Trashed, ext,
};
pub use refusal::Refusal;
pub use registry::Registry;
