//! Progress and cancel for the crate's long work: the host app shares one
//! [`Progress`] among the operations it reports on, and this crate counts
//! bytes and items into it, sets the phase and stops when cancelled. The
//! types live in the `places` crate, which every provider shares.

pub use places::{CANCELLED, Phase, Progress, Snapshot};
