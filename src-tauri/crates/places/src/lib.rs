//! One interface for every place File Browser shows (this computer, Google
//! Drive, Arbesk, SSH servers, received shares): the types every place
//! returns and the helpers they share, so no provider reaches into another
//! one or into the app for them.
//!
//! See `docs/superpowers/specs/2026-11-10-provider-crates-design.md`. This
//! crate depends on nothing but `serde`; no crate but the app depends on
//! Tauri.

mod entry;
mod files;
mod info;
mod progress;
mod refusal;

pub use entry::{CloudEntry, Entry, OpenTarget};
pub use files::{MARK_SUFFIX, is_unchanged, mark, mark_path, now, unchanged, write_json};
pub use info::{FileProperties, Measure, PathInfo};
pub use progress::{
    CANCELLED, Phase, Progress, SLICE, Snapshot, cancelled_io, copy_counted, is_cancel,
};
pub use refusal::Refusal;
