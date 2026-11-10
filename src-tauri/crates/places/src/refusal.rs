//! Why a copy, move or download was refused before anything was written,
//! in the words the user sees. This computer's transfers (`placement.rs` in
//! the app) and Google Drive's share them.

use std::fmt;
use std::path::PathBuf;

/// Why a copy, move or download was refused before anything was written.
/// `Display` is the message the user sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The source is gone; it holds the source as shown (a path, or a
    /// Google Drive name).
    Vanished(String),
    SamePath,
    IntoItself,
    /// Something is at the target and Replace wasn't asked for; it holds the
    /// target as shown.
    Exists(String),
    /// Replace would send the source's own folder to the Trash.
    ReplaceWithChild,
    /// The target's last component is empty, `.` or `..`, so Replace would
    /// act on its folder instead.
    NotAName(PathBuf),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Vanished(shown) => write!(f, "“{shown}” no longer exists"),
            Refusal::SamePath => f.write_str("The source and destination are the same"),
            Refusal::IntoItself => f.write_str("Cannot copy or move a folder into itself"),
            Refusal::Exists(shown) => write!(f, "Target already exists: {shown}"),
            Refusal::ReplaceWithChild => {
                f.write_str("Cannot replace a folder with an item inside it")
            }
            Refusal::NotAName(to) => {
                write!(f, "“{}” is not a valid name for a local item.", to.display())
            }
        }
    }
}

impl From<Refusal> for String {
    fn from(refusal: Refusal) -> String {
        refusal.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusals_read_as_they_always_have() {
        for (refusal, text) in [
            (Refusal::Vanished("/a/b".into()), "“/a/b” no longer exists"),
            (Refusal::SamePath, "The source and destination are the same"),
            (Refusal::IntoItself, "Cannot copy or move a folder into itself"),
            (Refusal::Exists("a.txt".into()), "Target already exists: a.txt"),
            (Refusal::ReplaceWithChild, "Cannot replace a folder with an item inside it"),
            (Refusal::NotAName("/a/..".into()), "“/a/..” is not a valid name for a local item."),
        ] {
            assert_eq!(String::from(refusal), text);
        }
    }
}
